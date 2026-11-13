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
    let data_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("NDM Next");
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
        let request: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
        let kind = request.get("type").and_then(Value::as_str);
        let mut result = if matches!(kind, Some("status" | "prepare")) && !executable.is_file() {
            json!({"ok":false,"error":"desktop_unavailable"})
        } else {
            host.handle(request.clone())
        };
        if kind == Some("commit") && result.get("ok") == Some(&Value::Bool(true)) {
            // The window process owns the scheduler. A second launch wakes the
            // existing process and exits without creating another scheduler.
            let launched = Command::new(&executable)
                .arg("--handoff")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if launched.is_err() {
                // The task remains durable and can be retried after the app is restored.
                result = json!({"ok":false,"error":"desktop_unavailable"});
            }
        }
        let response = serde_json::to_vec(&result).map_err(io::Error::other)?;
        write_frame(&mut output, &response)?;
    }
    Ok(())
}

fn main() {
    // stdout is exclusively reserved for Native Messaging frames; neither
    // malformed requests nor task credentials are written to diagnostic logs.
    let _ = run();
}
