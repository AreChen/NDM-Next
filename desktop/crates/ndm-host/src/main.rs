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

mod mcp;

#[derive(Deserialize)]
struct HostPreferences {
    download_dir: Option<PathBuf>,
    use_system_download_dir: Option<bool>,
    max_active_tasks: Option<u32>,
    max_connections: Option<u32>,
}

fn read_preferences(data_dir: &std::path::Path) -> Option<HostPreferences> {
    fs::read(data_dir.join("preferences.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<HostPreferences>(&bytes).ok())
}

fn configured_downloads(data_dir: &std::path::Path) -> PathBuf {
    read_preferences(data_dir)
        .map(|prefs| {
            ndm_core::resolve_download_directory(
                prefs.download_dir.as_deref(),
                prefs.use_system_download_dir,
            )
            .0
        })
        .unwrap_or_else(ndm_core::system_download_directory)
}

fn configured_limits(data_dir: &std::path::Path) -> ndm_core::DownloadLimits {
    let defaults = ndm_core::DownloadLimits::default();
    read_preferences(data_dir).map_or(defaults, |prefs| ndm_core::DownloadLimits {
        active_tasks: prefs
            .max_active_tasks
            .unwrap_or(defaults.active_tasks)
            .clamp(1, 32),
        download_threads: prefs
            .max_connections
            .unwrap_or(defaults.download_threads)
            .clamp(1, 256),
    })
}

fn run() -> io::Result<()> {
    let data_dir = data_directory();
    if ndm_core::desktop_installation_active(&data_dir)? {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "installation is in progress",
        ));
    }
    let store = Arc::new(Store::open(&data_dir.join("tasks.db")).map_err(io::Error::other)?);
    let executable = std::env::current_exe()?.with_file_name(if cfg!(windows) {
        "ndm-desktop.exe"
    } else {
        "ndm-desktop"
    });
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    while let Some(message) = read_frame(&mut input)? {
        if ndm_core::desktop_installation_active(&data_dir)? {
            return Ok(());
        }
        let host = Host::new(store.clone(), configured_downloads(&data_dir));
        let request: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
        let kind = request.get("type").and_then(Value::as_str);
        let can_commit = if matches!(kind, Some("commit" | "discard")) {
            let launched = Command::new(&executable)
                .arg("--handoff")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
            {
                launched.is_ok()
                    && ndm_core::wait_for_desktop_ready(
                        &data_dir,
                        std::time::Duration::from_secs(20),
                    )
                    .unwrap_or(false)
            }
            #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
            {
                launched.is_ok()
            }
        } else {
            true
        };
        let mut result = if (matches!(kind, Some("status" | "browser_context" | "prepare"))
            && !executable.is_file())
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
            #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
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
    ndm_core::application_data_directory()
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

fn preserve_manifest_origins(
    path: &std::path::Path,
    name: &str,
    expected_host: &std::path::Path,
    ids: &mut Vec<String>,
) {
    use std::io::Read;
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return;
    }
    let Ok(expected) = fs::canonicalize(expected_host) else {
        return;
    };
    let Ok(manifest_path) = fs::canonicalize(path) else {
        return;
    };
    if manifest_path.parent() != expected.parent() {
        return;
    }
    let Ok(file) = fs::File::open(path) else {
        return;
    };
    let mut contents = Vec::new();
    if file
        .take(1024 * 1024 + 1)
        .read_to_end(&mut contents)
        .is_err()
        || contents.len() > 1024 * 1024
    {
        return;
    }
    let Ok(existing) = serde_json::from_slice::<Value>(&contents) else {
        return;
    };
    let owner = existing
        .get("path")
        .and_then(Value::as_str)
        .map(std::path::Path::new);
    if existing.get("name").and_then(Value::as_str) != Some(name)
        || existing.get("type").and_then(Value::as_str) != Some("stdio")
        || !owner
            .filter(|path| path.is_absolute())
            .is_some_and(|path| fs::canonicalize(path).is_ok_and(|path| path == expected))
    {
        return;
    }
    let Some(origins) = existing.get("allowed_origins").and_then(Value::as_array) else {
        return;
    };
    for origin in origins {
        let Some(id) = origin
            .as_str()
            .and_then(|value| value.strip_prefix("chrome-extension://"))
            .and_then(|value| value.strip_suffix('/'))
        else {
            continue;
        };
        if ids.len() < 16
            && id.len() == 32
            && id.bytes().all(|value| (b'a'..=b'p').contains(&value))
            && !ids.iter().any(|known| known == id)
        {
            ids.push(id.to_owned());
        }
    }
}

fn old_installation_host(desktop: &std::path::Path) -> io::Result<Option<PathBuf>> {
    if !desktop.is_absolute()
        || !desktop
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("ndm-desktop.exe"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid previous installation image",
        ));
    }
    if !desktop.is_file() {
        return Ok(None);
    }
    let desktop = fs::canonicalize(desktop)?;
    let host = desktop.with_file_name("ndm-host.exe");
    if !host.is_file() {
        return Ok(None);
    }
    let canonical_host = fs::canonicalize(&host)?;
    if canonical_host.parent() != desktop.parent()
        || !canonical_host
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("ndm-host.exe"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "previous native host is outside its installation",
        ));
    }
    Ok(Some(host))
}

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--mcp")
    {
        let result = helper_data_directory(&arguments[1..]).and_then(mcp::run);
        if result.is_err() {
            eprintln!("NDM Next MCP server could not start or lost its transport");
        }
        std::process::exit(i32::from(result.is_err()));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if arguments
        .first()
        .is_some_and(|argument| argument == "--register-browser-hosts")
    {
        let result = std::env::current_exe()
            .and_then(|host| ndm_host::browser::register_browser_hosts(&host, &arguments[1..]));
        if let Err(error) = &result {
            eprintln!("{error}");
        }
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
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
            let mut index = 2;
            let preserve = arguments
                .get(index)
                .is_some_and(|arg| arg == "--preserve-origins");
            if preserve {
                index += 1;
            }
            let migrated_host = if arguments
                .get(index)
                .is_some_and(|arg| arg == "--migrate-origins-from")
            {
                if !preserve {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "migration requires preserved origins",
                    ));
                }
                let desktop = arguments.get(index + 1).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "missing previous installation image",
                    )
                })?;
                index += 2;
                old_installation_host(std::path::Path::new(desktop))?
            } else {
                None
            };
            let mut ids = arguments[index..].to_vec();
            // Validate the requested configuration even if an existing file is present.
            ndm_host::native_manifest(name, &executable, &ids)?;
            if preserve {
                preserve_manifest_origins(&destination, name, &executable, &mut ids);
                if let Some(host) = migrated_host {
                    preserve_manifest_origins(
                        &host.with_file_name("native-messaging-host.json"),
                        name,
                        &host,
                        &mut ids,
                    );
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

#[cfg(test)]
mod storage_tests {
    use super::configured_downloads;
    #[test]
    fn storage_preferences_are_reloaded_for_each_message() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["first", "第二个目录"] {
            let expected = directory.path().join(name);
            std::fs::write(directory.path().join("preferences.json"), serde_json::to_vec(&serde_json::json!({"download_dir": expected, "use_system_download_dir": false})).unwrap()).unwrap();
            assert_eq!(configured_downloads(directory.path()), expected);
        }
    }
}
