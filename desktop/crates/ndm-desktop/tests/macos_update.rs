#![cfg(target_os = "macos")]

use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn write_plist(bundle: &Path, version: &Version) {
    fs::write(
        bundle.join("Contents/Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.ndmnext.desktop</string>
<key>CFBundleExecutable</key><string>ndm-desktop</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
</dict></plist>"#
        ),
    )
    .unwrap();
}

fn checked(command: &mut Command) {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn invoke(executable: &Path, profile: &Path, dmg: &Path, identity: &str) -> Output {
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(profile.join("updating.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    Command::new(executable)
        .arg("--install-macos-update")
        .arg(profile)
        .arg(dmg)
        .arg(identity)
        .stdin(lock)
        .output()
        .unwrap()
}

#[test]
fn verified_dmg_replaces_and_relaunches_the_app_in_place() {
    let root = tempfile::tempdir().unwrap();
    let current_version = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
    let next_version = Version::new(
        current_version.major,
        current_version.minor,
        current_version.patch + 1,
    );
    let installed = root.path().join("My OnesDM.app");
    let executable = installed.join("Contents/MacOS/ndm-desktop");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_ndm-desktop"), &executable).unwrap();
    write_plist(&installed, &current_version);

    let source = root.path().join("image");
    let candidate = source.join("OnesDM.app");
    let new_binary = candidate.join("Contents/MacOS/ndm-desktop");
    fs::create_dir_all(new_binary.parent().unwrap()).unwrap();
    write_plist(&candidate, &next_version);
    let mut compiler = Command::new("/usr/bin/cc")
        .args(["-x", "c", "-", "-o"])
        .arg(&new_binary)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    compiler
        .stdin
        .take()
        .unwrap()
        .write_all(
            b"#include <stdio.h>\nint main(void) { puts(\"updated application launched\"); }\n",
        )
        .unwrap();
    assert!(compiler.wait().unwrap().success());
    let host = new_binary.with_file_name("ndm-host");
    fs::copy(&new_binary, &host).unwrap();
    for path in [&host, &new_binary, &candidate] {
        checked(
            Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(path),
        );
    }
    let dmg = root.path().join("update.dmg");
    checked(
        Command::new("/usr/bin/hdiutil")
            .args(["create", "-fs", "HFS+", "-format", "UDZO", "-srcfolder"])
            .arg(&source)
            .arg(&dmg),
    );
    let image = fs::read(&dmg).unwrap();
    let digest: [u8; 32] = Sha256::digest(&image).into();
    let identity = |digest: [u8; 32]| {
        serde_json::to_string(&(next_version.to_string(), image.len(), digest)).unwrap()
    };

    let profile = root.path().join("profile");
    fs::create_dir(&profile).unwrap();
    fs::write(profile.join("preferences.json"), b"stored settings").unwrap();
    let original_plist = fs::read(installed.join("Contents/Info.plist")).unwrap();
    let mut wrong_digest = digest;
    wrong_digest[0] ^= 1;
    let rejected = invoke(&executable, &profile, &dmg, &identity(wrong_digest));
    assert_eq!(rejected.status.code(), Some(3));
    assert_eq!(
        fs::read(installed.join("Contents/Info.plist")).unwrap(),
        original_plist
    );

    let result = invoke(&executable, &profile, &dmg, &identity(digest));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"updated application launched\n");
    assert_eq!(
        fs::read(installed.join("Contents/Info.plist")).unwrap(),
        fs::read(candidate.join("Contents/Info.plist")).unwrap()
    );
    assert_eq!(
        fs::read(profile.join("preferences.json")).unwrap(),
        b"stored settings"
    );
    assert!(!ndm_core::desktop_installation_active(&profile).unwrap());
}
