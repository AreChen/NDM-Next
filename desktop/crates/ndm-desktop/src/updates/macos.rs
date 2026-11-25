use super::{MAX_INSTALLER, ReadyUpdate, UpdateFailure};
use ndm_core::DesktopUpdateGuard;
use rustix::fs::{CWD, RenameFlags, renameat_with};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::AsFd,
        unix::{fs::MetadataExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const INSTALL: &str = "--install-macos-update";
const FINISH: &str = "--finish-macos-update";
pub(crate) const UPDATE_ROLLED_BACK: &str = "--macos-update-rolled-back";
const STAGING_PREFIX: &str = ".ndm-next-update-";
const BUNDLE_NAME: &str = "NDM Next.app";
const DESKTOP_IMAGE: &str = "Contents/MacOS/ndm-desktop";
const EXIT_MANUAL: i32 = 2;
const EXIT_INTEGRITY: i32 = 3;

type ImageIdentity = (String, u64, [u8; 32]);

enum InstallError {
    Manual,
    Integrity,
    Io(io::Error),
}

impl From<io::Error> for InstallError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) async fn install(
    update: ReadyUpdate,
    data_dir: PathBuf,
    reservation: DesktopUpdateGuard,
) -> Result<(), UpdateFailure> {
    let executable =
        std::env::current_exe().map_err(|error| UpdateFailure::Launch(error.raw_os_error()))?;
    let identity = serde_json::to_string(&(
        update.release.version.to_string(),
        update.release.bytes,
        update.release.sha256,
    ))
    .map_err(|_| UpdateFailure::Integrity)?;
    let mut command = tokio::process::Command::new(executable);
    command
        .arg(INSTALL)
        .arg(data_dir)
        .arg(&update.path)
        .arg(identity)
        .stdin(reservation.into_installer_stdin())
        .stdout(Stdio::null())
        .kill_on_drop(false);
    let mut child = command
        .spawn()
        .map_err(|error| UpdateFailure::Launch(error.raw_os_error()))?;
    drop(command);
    let status = child
        .wait()
        .await
        .map_err(|error| UpdateFailure::Launch(error.raw_os_error()))?;
    // The helper takes over on success. Only the manual fallback returns to
    // the original desktop with a successful installation handoff.
    match status.code() {
        Some(EXIT_MANUAL) => {
            let status = tokio::process::Command::new("/usr/bin/open")
                .arg("--")
                .arg(&update.path)
                .status()
                .await
                .map_err(|error| UpdateFailure::Launch(error.raw_os_error()))?;
            if status.success() {
                Ok(())
            } else {
                Err(UpdateFailure::Installer(status.code()))
            }
        }
        Some(EXIT_INTEGRITY) => Err(UpdateFailure::Integrity),
        code => Err(UpdateFailure::Installer(code)),
    }
}

pub(crate) fn run_installer() {
    let mut arguments = std::env::args_os().skip(1);
    if !arguments.next().is_some_and(|argument| argument == INSTALL) {
        return;
    }
    let result = run(arguments.collect());
    let code = match result {
        Ok(()) => 0,
        Err(InstallError::Manual) => EXIT_MANUAL,
        Err(InstallError::Integrity) => EXIT_INTEGRITY,
        Err(InstallError::Io(error)) => {
            eprintln!("NDM Next upgrade did not finish: {error}");
            1
        }
    };
    std::process::exit(code);
}

fn run(arguments: Vec<OsString>) -> Result<(), InstallError> {
    let [data_dir, image, identity]: [OsString; 3] = arguments
        .try_into()
        .map_err(|_| io::Error::other("invalid installer arguments"))?;
    let data_dir = PathBuf::from(data_dir);
    let (version, bytes, digest): ImageIdentity =
        serde_json::from_str(identity.to_str().ok_or(InstallError::Integrity)?)
            .map_err(|_| InstallError::Integrity)?;
    let version = Version::parse(&version).map_err(|_| InstallError::Integrity)?;
    if bytes == 0 || bytes > MAX_INSTALLER {
        return Err(InstallError::Integrity);
    }
    // The inherited file shares the GUI's locked open file description. Keep
    // a close-on-exec copy so a failed exec still leaves rollback protected.
    let reservation = File::from(std::io::stdin().as_fd().try_clone_to_owned()?);
    let lock = reservation.metadata()?;
    let expected = fs::symlink_metadata(data_dir.join("updating.lock"))?;
    if !expected.is_file() || lock.dev() != expected.dev() || lock.ino() != expected.ino() {
        return Err(io::Error::other("missing update reservation").into());
    }
    reservation
        .try_lock()
        .map_err(|error| io::Error::other(error.to_string()))?;

    let executable = std::env::current_exe()?.canonicalize()?;
    let current = bundle_for(&executable).ok_or(InstallError::Manual)?;
    let installed_version = bundle_version(current)?;
    if installed_version.to_string() != env!("CARGO_PKG_VERSION") || version <= installed_version {
        return Err(InstallError::Integrity);
    }
    let directory = current.parent().ok_or(InstallError::Manual)?;
    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(directory)
        .map_err(|error| match error.kind() {
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                InstallError::Manual
            }
            _ => InstallError::Io(error),
        })?;
    let incoming = stage_image(Path::new(&image), bytes, digest, staging.path())?;
    verify_bundle(&incoming, &version).map_err(|_| InstallError::Integrity)?;

    // The normal quit handler checkpoints the scheduler and tears down the
    // desktop singleton. The installer retains download admission until exec.
    if ndm_core::desktop_is_running(&data_dir)? {
        let _ = ndm_core::request_desktop_shutdown(&data_dir);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while ndm_core::desktop_is_running(&data_dir)? {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "desktop did not exit").into());
        }
        thread::sleep(Duration::from_millis(50));
    }

    match exchange(current, &incoming) {
        Ok(()) => {
            let error = relaunch(&executable)
                .arg(FINISH)
                .arg(
                    staging
                        .path()
                        .file_name()
                        .expect("temporary directory name"),
                )
                .exec();
            eprintln!("Could not launch upgraded NDM Next: {error}");
            if let Err(error) = exchange(current, &incoming) {
                let backup = staging.keep();
                eprintln!("NDM Next backup retained at {}", backup.display());
                return Err(error.into());
            }
        }
        Err(error) => eprintln!("Could not replace NDM Next: {error}"),
    }
    drop(staging);
    Err(relaunch(&executable).arg(UPDATE_ROLLED_BACK).exec().into())
}

fn relaunch(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.stdin(Stdio::null());
    command
}

fn checked_status(command: &mut Command) -> io::Result<()> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).spawn()?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "{} failed ({status})",
                    command.get_program().to_string_lossy()
                )))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "macOS updater command timed out",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn stage_image(
    source: &Path,
    bytes: u64,
    digest: [u8; 32],
    staging: &Path,
) -> Result<PathBuf, InstallError> {
    let image_workspace = tempfile::tempdir()?;
    let image = image_workspace.path().join("update.dmg");
    // Only mount bytes we have verified in this private directory. The cache
    // can reside outside the user's profile on a different volume.
    if !fs::symlink_metadata(source)?.is_file() {
        return Err(InstallError::Integrity);
    }
    let mut source = File::open(source)?.take(bytes + 1);
    let mut output = File::create(&image)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut copied = 0;
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        copied += read as u64;
    }
    if copied != bytes || <[u8; 32]>::from(hasher.finalize()) != digest {
        return Err(InstallError::Integrity);
    }
    drop(output);
    let mount = image_workspace.path().join("mount");
    fs::create_dir(&mount)?;
    let original_device = fs::metadata(&mount)?.dev();
    let incoming = staging.join(BUNDLE_NAME);
    let attached = checked_status(
        Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-readonly",
                "-nobrowse",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(&image),
    );
    let mounted =
        attached.is_ok() || fs::metadata(&mount).is_ok_and(|meta| meta.dev() != original_device);
    let result = attached.and_then(|()| {
        let app = mount.join("OnesDM.app");
        let app = if app.try_exists()? {
            app
        } else {
            mount.join("NDM Next.app")
        };
        if !fs::symlink_metadata(&app)?.is_dir() {
            return Err(invalid_bundle());
        }
        checked_status(Command::new("/usr/bin/ditto").arg(app).arg(&incoming))
    });
    // A failed attach may still have mounted the volume. Detach before
    // removing the private directory, even if copying the bundle failed.
    if mounted
        && let Err(error) =
            checked_status(Command::new("/usr/bin/hdiutil").arg("detach").arg(&mount))
    {
        let _ = image_workspace.keep();
        return Err(error.into());
    }
    result?;
    Ok(incoming)
}

fn bundle_for(executable: &Path) -> Option<&Path> {
    let path = executable.ancestors().nth(3)?;
    (path.extension().is_some_and(|suffix| suffix == "app")
        && executable.strip_prefix(path).ok()? == Path::new(DESKTOP_IMAGE))
    .then_some(path)
}

fn invalid_bundle() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid NDM Next application bundle",
    )
}

fn bundle_version(bundle: &Path) -> io::Result<Version> {
    let contents = bundle.join("Contents");
    let plist = contents.join("Info.plist");
    if !fs::symlink_metadata(bundle)?.is_dir()
        || !fs::symlink_metadata(&contents)?.is_dir()
        || !fs::symlink_metadata(&plist)?.is_file()
    {
        return Err(invalid_bundle());
    }
    let field = |name| -> io::Result<String> {
        let output = Command::new("/usr/bin/plutil")
            .args(["-extract", name, "raw", "-o", "-"])
            .arg(&plist)
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(invalid_bundle());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    if field("CFBundleIdentifier")? != "com.ndmnext.desktop"
        || field("CFBundleExecutable")? != "ndm-desktop"
    {
        return Err(invalid_bundle());
    }
    Version::parse(&field("CFBundleShortVersionString")?).map_err(|_| invalid_bundle())
}

fn verify_bundle(bundle: &Path, version: &Version) -> io::Result<()> {
    if bundle_version(bundle)? != *version
        || !fs::symlink_metadata(bundle.join("Contents/MacOS"))?.is_dir()
    {
        return Err(invalid_bundle());
    }
    let cpu: u32 = if cfg!(target_arch = "aarch64") {
        0x0100_000c
    } else {
        0x0100_0007
    };
    for binary in [DESKTOP_IMAGE, "Contents/MacOS/ndm-host"] {
        let path = bundle.join(binary);
        let metadata = fs::symlink_metadata(&path)?;
        let mut header = [0; 8];
        if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
            return Err(invalid_bundle());
        }
        File::open(path)?.read_exact(&mut header)?;
        if header[..4] != [0xcf, 0xfa, 0xed, 0xfe] || header[4..] != cpu.to_le_bytes() {
            return Err(invalid_bundle());
        }
    }
    checked_status(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(bundle),
    )
}

fn exchange(current: &Path, incoming: &Path) -> io::Result<()> {
    renameat_with(CWD, current, CWD, incoming, RenameFlags::EXCHANGE).map_err(Into::into)
}

pub(crate) fn finish_update() {
    let mut arguments = std::env::args_os().skip(1);
    if !arguments.next().is_some_and(|argument| argument == FINISH) {
        return;
    }
    let Some(name) = arguments.next() else {
        return;
    };
    if !name.to_str().is_some_and(|name| {
        name.starts_with(STAGING_PREFIX) && Path::new(name).file_name() == Some(OsStr::new(name))
    }) {
        return;
    }
    thread::spawn(move || {
        let result = (|| -> io::Result<()> {
            let executable = std::env::current_exe()?.canonicalize()?;
            let current = bundle_for(&executable).ok_or_else(invalid_bundle)?;
            let staging = current.parent().ok_or_else(invalid_bundle)?.join(name);
            let backup = staging.join(BUNDLE_NAME);
            let running_version = bundle_version(current)?;
            if !fs::symlink_metadata(&staging)?.is_dir()
                || running_version.to_string() != env!("CARGO_PKG_VERSION")
                || bundle_version(&backup)? >= running_version
            {
                return Err(invalid_bundle());
            }
            fs::remove_dir_all(backup)?;
            fs::remove_dir(staging)
        })();
        if let Err(error) = result {
            eprintln!("Could not remove the previous NDM Next bundle: {error}");
        }
    });
}
