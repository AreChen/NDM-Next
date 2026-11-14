use std::{
    fs, io,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
};

use ndm_core::Store;
use ndm_host::{Host, read_frame, write_frame};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct HostPreferences {
    download_dir: Option<PathBuf>,
}

fn default_downloads() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Downloads")
}

fn run() -> io::Result<()> {
    let data_dir = data_directory();
    if ndm_core::desktop_installation_active(&data_dir)? {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "installation is in progress",
        ));
    }
    let destination = fs::read(data_dir.join("preferences.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<HostPreferences>(&bytes).ok())
        .and_then(|prefs| prefs.download_dir)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(default_downloads);
    let store = Store::open(&data_dir.join("tasks.db")).map_err(io::Error::other)?;
    let host = Host::new(Arc::new(store), destination);
    let executable = std::env::current_exe()?.with_file_name("ndm-desktop.exe");
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    while let Some(message) = read_frame(&mut input)? {
        if ndm_core::desktop_installation_active(&data_dir)? {
            return Ok(());
        }
        let request: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
        let kind = request.get("type").and_then(Value::as_str);
        let can_commit = if matches!(kind, Some("commit" | "discard")) {
            let launched = Command::new(&executable)
                .arg("--handoff")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            #[cfg(windows)]
            {
                launched.is_ok()
                    && ndm_core::wait_for_desktop_ready(
                        &data_dir,
                        std::time::Duration::from_secs(20),
                    )
                    .unwrap_or(false)
            }
            #[cfg(not(windows))]
            {
                launched.is_ok()
            }
        } else {
            true
        };
        let mut result = if (matches!(kind, Some("status" | "prepare")) && !executable.is_file())
            || !can_commit
        {
            json!({"ok":false,"error":"desktop_unavailable"})
        } else {
            host.handle(request.clone())
        };
        if matches!(kind, Some("commit" | "discard"))
            && result.get("ok") == Some(&Value::Bool(true))
        {
            // Commit only after startup recovery finishes, then wake the scheduler.
            // A retry is safe if the desktop exits between these two operations.
            #[cfg(windows)]
            if ndm_core::wake_desktop(&data_dir).is_err() {
                result = json!({"ok":false,"error":"desktop_unavailable"});
            }
        }
        let response = serde_json::to_vec(&result).map_err(io::Error::other)?;
        write_frame(&mut output, &response)?;
    }
    Ok(())
}

fn data_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("NDM Next")
}

fn helper_data_directory(arguments: &[String]) -> io::Result<PathBuf> {
    match arguments {
        [] => Ok(data_directory()),
        [flag, path] if flag == "--data-dir" && PathBuf::from(path).is_absolute() => {
            Ok(PathBuf::from(path))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid data directory",
        )),
    }
}

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if arguments
        .first()
        .is_some_and(|arg| arg == "--shutdown-desktop")
    {
        let result = (|| {
            let expected = arguments.get(1).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "expected desktop image")
            })?;
            let data_dir = helper_data_directory(&arguments[2..])?;
            ndm_core::shutdown_desktop_installation(
                &data_dir,
                std::path::Path::new(expected),
                std::time::Duration::from_secs(30),
            )
        })();
        if let Err(error) = &result {
            eprintln!("{error}");
        }
        // Installer may try its current path only when a different image owns
        // the profile. A real graceful timeout must never become legacy fallback.
        std::process::exit(match result {
            Ok(()) => 0,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => 3,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => 4,
            Err(_) => 2,
        });
    }
    if arguments
        .first()
        .is_some_and(|arg| arg == "--installation-namespace")
    {
        let result = helper_data_directory(&arguments[1..]).and_then(|data_dir| {
            use std::io::Write;
            io::stdout().write_all(ndm_core::desktop_installation_namespace(&data_dir).as_bytes())
        });
        std::process::exit(if result.is_ok() { 0 } else { 2 });
    }
    #[cfg(windows)]
    if arguments
        .first()
        .is_some_and(|arg| arg == "--desktop-running")
    {
        let data_dir = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("NDM Next");
        std::process::exit(match ndm_core::desktop_is_running(&data_dir) {
            Ok(false) => 0,
            Ok(true) => 1,
            Err(_) => 2,
        });
    }
    if arguments
        .first()
        .is_some_and(|arg| arg == "--write-manifest")
    {
        let result = (|| -> io::Result<()> {
            let executable = std::env::current_exe()?;
            let name = arguments
                .get(1)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing host name"))?;
            let destination = executable.with_file_name("native-messaging-host.json");
            let preserve = arguments
                .get(2)
                .is_some_and(|arg| arg == "--preserve-origins");
            let mut ids = arguments[if preserve { 3 } else { 2 }..].to_vec();
            // Validate the requested configuration even if an existing file is present.
            ndm_host::native_manifest(name, &executable, &ids)?;
            if preserve
                && fs::metadata(&destination).is_ok_and(|metadata| metadata.len() <= 1024 * 1024)
                && let Ok(contents) = fs::read(&destination)
                && let Ok(existing) = serde_json::from_slice::<Value>(&contents)
                && existing.get("name").and_then(Value::as_str) == Some(name.as_str())
                && existing.get("path").and_then(Value::as_str) == executable.to_str()
                && existing.get("type").and_then(Value::as_str) == Some("stdio")
                && let Some(origins) = existing.get("allowed_origins").and_then(Value::as_array)
            {
                for origin in origins {
                    let Some(id) = origin
                        .as_str()
                        .and_then(|origin| origin.strip_prefix("chrome-extension://"))
                        .and_then(|origin| origin.strip_suffix('/'))
                    else {
                        continue;
                    };
                    if ids.len() < 16
                        && id.len() == 32
                        && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
                        && !ids.iter().any(|known| known == id)
                    {
                        ids.push(id.to_owned());
                    }
                }
            }
            let manifest = ndm_host::native_manifest(name, &executable, &ids)?;
            let mut temporary = tempfile::NamedTempFile::new_in(
                destination
                    .parent()
                    .ok_or_else(|| io::Error::other("missing installation directory"))?,
            )?;
            serde_json::to_writer_pretty(&mut temporary, &manifest).map_err(io::Error::other)?;
            temporary.persist(destination).map_err(io::Error::other)?;
            Ok(())
        })();
        if result.is_err() {
            std::process::exit(1);
        }
        return;
    }
    // stdout is exclusively reserved for Native Messaging frames; neither
    // malformed requests nor task credentials are written to diagnostic logs.
    let _ = run();
}
