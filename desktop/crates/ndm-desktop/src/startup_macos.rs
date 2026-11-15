use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn path() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is missing"))?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents/com.ndmnext.desktop.plist"))
}

fn launch_agent(executable: &Path) -> io::Result<String> {
    let executable = executable
        .to_str()
        .ok_or_else(|| io::Error::other("invalid app path"))?;
    let escaped = executable
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.ndmnext.desktop</string>
<key>ProgramArguments</key><array><string>{escaped}</string><string>--background</string></array>
<key>RunAtLoad</key><true/>
<key>LimitLoadToSessionType</key><string>Aqua</string>
</dict></plist>
"#
    ))
}

pub fn enabled() -> io::Result<bool> {
    match fs::read_to_string(path()?) {
        Ok(contents) => Ok(contents == launch_agent(&std::env::current_exe()?)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn set(enabled: bool) -> io::Result<()> {
    let path = path()?;
    if enabled {
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("missing LaunchAgents directory"))?;
        fs::create_dir_all(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        use io::Write;
        file.write_all(launch_agent(&std::env::current_exe()?)?.as_bytes())?;
        file.persist(path).map_err(io::Error::other)?;
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
    fn launch_agent_escapes_app_paths_and_starts_in_background() {
        let xml = super::launch_agent(std::path::Path::new(
            "/Applications/NDM & Next.app/Contents/MacOS/ndm-desktop",
        ))
        .unwrap();
        assert!(xml.contains("NDM &amp; Next.app"));
        assert!(xml.contains("<string>--background</string>"));
        assert!(!xml.contains("KeepAlive"));
    }
}
