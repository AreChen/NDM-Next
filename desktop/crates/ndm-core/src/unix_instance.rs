//! Per-profile Unix process coordination. File locks release automatically on exit.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{fs::OpenOptionsExt, net::UnixStream},
    path::Path,
    time::{Duration, Instant},
};

pub fn lock_file(data_dir: &Path, name: &str) -> io::Result<File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(data_dir)?;
        let metadata = std::fs::symlink_metadata(data_dir)?;
        // SAFETY: geteuid reads the effective UID of this process.
        let uid = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::other(
                "desktop profile has unsafe ownership or permissions",
            ));
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(data_dir.join(name))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::other(
                "desktop lock has unsafe ownership or permissions",
            ));
        }
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::fs::create_dir_all(data_dir)?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(data_dir.join(name))
    }
}

pub fn socket_path(data_dir: &Path) -> io::Result<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        use sha2::{Digest, Sha256};
        use std::{
            os::unix::{ffi::OsStrExt, fs::MetadataExt},
            path::PathBuf,
        };
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            && let Ok(meta) = std::fs::symlink_metadata(&runtime)
            // SAFETY: geteuid reads the effective UID of this process.
            && meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0
        {
            let digest = Sha256::digest(data_dir.canonicalize()?.as_os_str().as_bytes());
            let name = format!("ndm-next-{digest:x}");
            return Ok(runtime.join(format!("{}.sock", &name[..25])));
        }
    }
    Ok(data_dir.join("desktop.sock"))
}

pub fn request(data_dir: &Path, command: u8) -> io::Result<bool> {
    let mut stream = UnixStream::connect(socket_path(data_dir)?)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    stream.write_all(&[command])?;
    let mut response = [0];
    stream.read_exact(&mut response)?;
    Ok(response[0] == 1)
}

pub fn desktop_is_running(data_dir: &Path) -> io::Result<bool> {
    let file = lock_file(data_dir, "desktop.lock")?;
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

pub fn wait_for_desktop_ready(data_dir: &Path, timeout: Duration) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if request(data_dir, b'R').unwrap_or(false) {
            return Ok(true);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(25)));
    }
}

pub fn wake_desktop(data_dir: &Path) -> io::Result<()> {
    if request(data_dir, b'W')? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "desktop is not ready",
        ))
    }
}

pub fn request_desktop_shutdown(data_dir: &Path) -> io::Result<bool> {
    request(data_dir, b'Q')
}
