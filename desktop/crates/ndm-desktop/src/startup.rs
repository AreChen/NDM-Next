use std::{
    io,
    path::{Path, PathBuf},
};
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "NDM Next";
const INSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next";

// A local preview controls the installed application's startup entry, rather
// than replacing it with a temporary build that may disappear tomorrow.
fn startup_executable_at(install_key: &str, current: PathBuf) -> io::Result<PathBuf> {
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(install_key) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(current),
        Err(error) => return Err(error),
    };
    let directory = match key.get_value::<String, _>("InstallLocation") {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(current),
        Err(error) => return Err(error),
    };
    let installed = PathBuf::from(directory).join("ndm-desktop.exe");
    Ok(if installed.is_absolute() && installed.is_file() {
        installed
    } else {
        current
    })
}

fn command(executable: &Path) -> String {
    format!("\"{}\" --background", executable.display())
}

fn enabled_at(key_path: &str, executable: &Path) -> io::Result<bool> {
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key_path, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match key.get_value::<String, _>(VALUE) {
        Ok(value) => Ok(value == command(executable)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn set_at(key_path: &str, executable: &Path, enabled: bool) -> io::Result<()> {
    if enabled {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(key_path, KEY_READ | KEY_WRITE)?;
        match key.get_value::<String, _>(VALUE) {
            Ok(value) if value != command(executable) => {
                return Err(io::Error::other(
                    "startup entry belongs to another installation",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        key.set_value(VALUE, &command(executable))
    } else {
        let key = match RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(key_path, KEY_READ | KEY_WRITE)
        {
            Ok(key) => key,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        match key.get_value::<String, _>(VALUE) {
            Ok(value) if value == command(executable) => key.delete_value(VALUE),
            Ok(_) => Err(io::Error::other(
                "startup entry belongs to another installation",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub fn enabled() -> io::Result<bool> {
    enabled_at(
        RUN_KEY,
        &startup_executable_at(INSTALL_KEY, std::env::current_exe()?)?,
    )
}
pub fn set(enabled: bool) -> io::Result<()> {
    set_at(
        RUN_KEY,
        &startup_executable_at(INSTALL_KEY, std::env::current_exe()?)?,
        enabled,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn previews_control_the_registered_installation_and_preserve_unrelated_entries() {
        use super::*;
        let root = format!(r"Software\NDMNextTests\{}", uuid_for_test());
        let install_key = format!(r"{root}\Install");
        let run_key = format!(r"{root}\Run");
        let directory = tempfile::tempdir().unwrap();
        let installed = directory.path().join("ndm-desktop.exe");
        std::fs::write(&installed, []).unwrap();
        let current = directory.path().join("preview.exe");
        let (install, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&install_key)
            .unwrap();
        install
            .set_value("InstallLocation", &directory.path().to_str().unwrap())
            .unwrap();
        let resolved = startup_executable_at(&install_key, current.clone()).unwrap();
        assert_eq!(resolved, installed);
        set_at(&run_key, &installed, true).unwrap();
        assert!(enabled_at(&run_key, &resolved).unwrap());
        set_at(&run_key, &resolved, false).unwrap();
        set_at(&run_key, &current, true).unwrap();
        assert!(set_at(&run_key, &resolved, true).is_err());
        assert!(set_at(&run_key, &resolved, false).is_err());
        assert!(enabled_at(&run_key, &current).unwrap());
        std::fs::remove_file(installed).unwrap();
        assert_eq!(
            startup_executable_at(&install_key, current.clone()).unwrap(),
            current
        );
        RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey_all(root)
            .unwrap();
    }

    #[test]
    fn toggles_only_our_startup_value() {
        let registry_path = format!("Software\\NDMNextTests\\{}", uuid_for_test());
        let current = std::env::current_exe().unwrap();
        assert!(!super::enabled_at(&registry_path, &current).unwrap());
        super::set_at(&registry_path, &current, true).unwrap();
        assert!(super::enabled_at(&registry_path, &current).unwrap());
        super::set_at(&registry_path, &current, false).unwrap();
        assert!(!super::enabled_at(&registry_path, &current).unwrap());
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .delete_subkey(&registry_path)
            .unwrap();
        let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .delete_subkey(r"Software\NDMNextTests");
    }

    fn uuid_for_test() -> String {
        format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }
}
