use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};

fn path() -> PathBuf {
    ndm_core::linux_config_directory().join("autostart/com.ndmnext.desktop.desktop")
}

fn autostart_entry(executable: &Path) -> io::Result<String> {
    let executable = executable
        .to_str()
        .filter(|path| !path.chars().any(char::is_control))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid desktop path"))?;
    let mut argument = String::new();
    for ch in executable.chars() {
        match ch {
            '\\' => argument.push_str(r"\\\\"),
            '"' | '`' | '$' => {
                argument.push_str(r"\\");
                argument.push(ch);
            }
            '%' => argument.push_str("%%"),
            _ => argument.push(ch),
        }
    }
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=OnesDM\nExec=\"{argument}\" --background\nTerminal=false\n"
    ))
}

pub fn enabled() -> io::Result<bool> {
    match fs::read_to_string(path()) {
        Ok(contents) => Ok(contents == autostart_entry(&std::env::current_exe()?)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn set(enabled: bool) -> io::Result<()> {
    let path = path();
    if enabled {
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("missing autostart directory"))?;
        fs::create_dir_all(directory)?;
        let mut pending = tempfile::NamedTempFile::new_in(directory)?;
        pending.write_all(autostart_entry(&std::env::current_exe()?)?.as_bytes())?;
        pending.persist(path).map_err(io::Error::other)?;
    } else {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn autostart_quotes_the_path_and_escapes_desktop_field_codes() {
        let entry =
            super::autostart_entry(std::path::Path::new("/opt/OnesDM 100%/ndm-desktop")).unwrap();
        assert!(
            entry
                .lines()
                .any(|line| line == r#"Exec="/opt/OnesDM 100%%/ndm-desktop" --background"#)
        );
    }
}
