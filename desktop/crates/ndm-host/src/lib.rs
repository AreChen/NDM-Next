use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use ndm_core::{DashOptions, HlsOptions, NewTask, Store, TaskId, TaskOptions, TaskState};
use serde::Deserialize;
use serde_json::{Value, json};

/// Chromium accepts at most 1 MiB from a native host; apply the same limit both ways.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Build the manifest from the actual installed path, never a wildcard origin.
pub fn native_manifest(name: &str, executable: &Path, ids: &[String]) -> io::Result<Value> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid native host configuration",
        )
    };
    if name.is_empty()
        || name.len() > 255
        || name.split('.').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
        || ids.is_empty()
        || ids.len() > 16
        || ids
            .iter()
            .any(|id| id.len() != 32 || !id.bytes().all(|b| (b'a'..=b'p').contains(&b)))
        || !executable.is_absolute()
    {
        return Err(invalid());
    }
    let path = executable.to_str().ok_or_else(invalid)?;
    Ok(json!({
        "name": name,
        "description": "NDM Next browser integration",
        "path": path,
        "type": "stdio",
        "allowed_origins": ids.iter().map(|id| format!("chrome-extension://{id}/")).collect::<Vec<_>>(),
    }))
}

pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0_u8; 4];
    loop {
        match reader.read(&mut length[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    reader.read_exact(&mut length[1..])?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn write_frame(writer: &mut impl Write, message: &[u8]) -> io::Result<()> {
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    writer.write_all(&(message.len() as u32).to_le_bytes())?;
    writer.write_all(message)?;
    writer.flush()
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Status,
    Prepare { id: TaskId, task: BrowserTask },
    Commit { id: TaskId },
    Abort { id: TaskId },
    Discard { id: TaskId },
    Inspect { id: TaskId },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserTask {
    url: String,
    source_url: Option<String>,
    suggested_filename: Option<String>,
    expected_size: Option<u64>,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

impl BrowserTask {
    fn validate(self, destination: PathBuf) -> Option<(NewTask, TaskOptions)> {
        fn web_url(value: &str) -> bool {
            reqwest::Url::parse(value).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
        }
        if self.url.len() > 65_536
            || !web_url(&self.url)
            || self
                .source_url
                .as_ref()
                .is_some_and(|source| source.len() > 65_536 || !web_url(source))
            || self
                .suggested_filename
                .as_ref()
                .is_some_and(|name| name.len() > 1024 || name.chars().any(char::is_control))
            || self
                .media_type
                .as_ref()
                .is_some_and(|mime| mime.len() > 256 || mime.chars().any(char::is_control))
        {
            return None;
        }
        let mut headers = BTreeMap::new();
        let mut bytes = 0_usize;
        for (name, value) in self.headers {
            let name = name.to_ascii_lowercase();
            bytes = bytes.checked_add(name.len() + value.len())?;
            if !matches!(
                name.as_str(),
                "accept"
                    | "accept-language"
                    | "authorization"
                    | "cookie"
                    | "origin"
                    | "referer"
                    | "user-agent"
            ) || bytes > 65_536
                || reqwest::header::HeaderValue::from_str(&value).is_err()
                || headers.insert(name, value).is_some()
            {
                return None;
            }
        }
        let is_hls = self.media_type.as_deref().is_some_and(|mime| {
            matches!(
                mime.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "application/vnd.apple.mpegurl"
                    | "application/x-mpegurl"
                    | "audio/mpegurl"
                    | "audio/x-mpegurl"
            )
        });
        let is_dash = self.media_type.as_deref().is_some_and(|mime| {
            mime.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/dash+xml")
        });
        let options = if is_hls {
            TaskOptions::Hls(HlsOptions::default())
        } else if is_dash {
            TaskOptions::Dash(DashOptions::default())
        } else {
            TaskOptions::Auto
        };
        Some((
            NewTask {
                url: self.url,
                destination,
                suggested_filename: self.suggested_filename,
                source_url: self.source_url,
                expected_size: if is_hls || is_dash {
                    None
                } else {
                    self.expected_size
                },
                headers,
            },
            options,
        ))
    }
}

pub struct Host {
    store: Arc<Store>,
    destination: PathBuf,
}

fn state_name(state: TaskState) -> &'static str {
    match state {
        TaskState::Prepared => "prepared",
        TaskState::Queued => "queued",
        TaskState::Running => "running",
        TaskState::Paused => "paused",
        TaskState::Completed => "completed",
        TaskState::Failed => "failed",
    }
}

impl Host {
    pub fn new(store: Arc<Store>, destination: PathBuf) -> Self {
        Self { store, destination }
    }

    /// Replies intentionally contain no URL, cookies, or diagnostic text from input.
    pub fn handle(&self, message: Value) -> Value {
        let Ok(command) = serde_json::from_value::<Command>(message) else {
            return json!({"ok":false,"error":"invalid_request"});
        };
        match command {
            Command::Status => json!({"ok":true,"protocol":1}),
            Command::Prepare { id, task } => {
                // A retry of an existing handoff keeps its original destination even
                // when the application default changes between prepare requests.
                let destination = match self.store.get(id) {
                    Ok(Some(existing)) => existing.request.destination,
                    Ok(None) => self.destination.clone(),
                    Err(_) => return json!({"ok":false,"error":"prepare_failed"}),
                };
                let Some((task, options)) = task.validate(destination) else {
                    return json!({"ok":false,"error":"invalid_request"});
                };
                match self.store.prepare_with_options(id, &task, &options) {
                    Ok(task) => json!({"ok":true,"state":state_name(task.state)}),
                    Err(_) => json!({"ok":false,"error":"prepare_failed"}),
                }
            }
            Command::Commit { id } => match self.store.commit(id) {
                Ok(task) => json!({"ok":true,"state":state_name(task.state)}),
                Err(_) => json!({"ok":false,"error":"commit_failed"}),
            },
            Command::Abort { id } => match self.store.abort(id) {
                Ok(()) => json!({"ok":true}),
                Err(_) => json!({"ok":false,"error":"abort_failed"}),
            },
            Command::Discard { id } => match self.store.request_discard(id) {
                Ok(state) => json!({"ok":true,"state":state.map(state_name).unwrap_or("missing")}),
                Err(_) => json!({"ok":false,"error":"discard_failed"}),
            },
            Command::Inspect { id } => match self.store.get(id) {
                Ok(Some(task)) => {
                    let output_size = task
                        .output_path
                        .as_ref()
                        .and_then(|path| std::fs::symlink_metadata(path).ok())
                        .filter(|metadata| metadata.file_type().is_file())
                        .map(|metadata| metadata.len());
                    json!({"ok":true,"state":state_name(task.state),"size_verified":task.size_verified,"output_size":output_size})
                }
                Ok(None) => json!({"ok":true,"state":"missing"}),
                Err(_) => json!({"ok":false,"error":"inspect_failed"}),
            },
        }
    }
}
