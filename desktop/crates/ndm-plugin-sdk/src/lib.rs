//! Wire types and framing for NDM Next download engine workers.

mod manifest;
mod worker;

pub use manifest::*;
pub use worker::{Engine, Worker, serve, serve_engine};

use std::{
    collections::{BTreeMap, HashSet},
    io::{self, BufRead, Read, Write},
    num::NonZeroU32,
    path::PathBuf,
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const ENGINE_PROTOCOL_VERSION: u32 = 1;
pub const ENGINE_ARGUMENT: &str = "--ndm-engine";
pub const MEDIA_TOOLS_ENV: &str = "NDM_MEDIA_TOOLS";
pub const RUNTIMES_ENV: &str = "NDM_PLUGIN_RUNTIMES";

/// Absolute path of an executable declared in this package's runtime_dependencies.
pub fn runtime_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var(RUNTIMES_ENV).ok()?;
    serde_json::from_str::<BTreeMap<String, PathBuf>>(&value)
        .ok()?
        .remove(name)
        .filter(|path| path.is_absolute())
}
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// Optional engine-owned state that can be retained after publishing a completed transfer.
pub const ENGINE_STATE_DIRECTORY: &str = "state";

// Sources and options can contain credentials. Keep them out of Debug output.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DownloadSource {
    Url {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    File {
        path: PathBuf,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub task_id: uuid::Uuid,
    pub source: DownloadSource,
    pub workspace: PathBuf,
    pub suggested_filename: Option<String>,
    pub expected_size: Option<u64>,
    pub download_threads: Option<NonZeroU32>,
    #[serde(default)]
    pub options: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SourceSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSelection {
    /// The immutable source revision returned by Inspect.
    pub revision: String,
    pub file_ids: Vec<u32>,
}

impl SourceSelection {
    pub fn valid(&self) -> bool {
        valid_revision(&self.revision)
            && !self.file_ids.is_empty()
            && self.file_ids.windows(2).all(|pair| pair[0] < pair[1])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceFiles {
    pub revision: String,
    pub files: Vec<SourceFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: u32,
    /// A display path, never opened by the host.
    pub path: String,
    pub size: u64,
}

impl SourceFiles {
    pub fn valid(&self) -> bool {
        let mut ids = HashSet::new();
        valid_revision(&self.revision)
            && !self.files.is_empty()
            && self.files.iter().all(|file| {
                ids.insert(file.id)
                    && !file.path.trim().is_empty()
                    && file.path.len() <= 4096
                    && !file.path.chars().any(char::is_control)
            })
    }
}

fn valid_revision(revision: &str) -> bool {
    !revision.is_empty()
        && revision.len() <= 128
        && revision.bytes().all(|ch| ch.is_ascii_graphic())
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    Hello {
        version: u32,
        #[serde(default)]
        task_updates: bool,
        #[serde(default)]
        peer_stats: bool,
    },
    Download {
        request: Box<DownloadRequest>,
    },
    Inspect {
        request: Box<DownloadRequest>,
    },
    Share {
        request: Box<DownloadRequest>,
        output: PathBuf,
    },
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferProgress {
    /// Bytes present in the workspace, including data restored from a checkpoint.
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Network bytes transferred since this worker started.
    pub transferred: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerStats {
    pub connected: u32,
    pub seeders: u32,
    pub connecting: u32,
    pub queued: u32,
    pub known: u32,
    /// Upload bytes since this worker started. The host accumulates attempts.
    pub uploaded: u64,
    pub upload_bytes_per_second: u64,
}

impl PeerStats {
    pub fn inactive(&mut self) {
        *self = Self {
            uploaded: self.uploaded,
            ..Self::default()
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    InvalidSource,
    Authentication,
    Network,
    Storage,
    Unsupported,
    InvalidOptions,
    PostProcessing,
    Other,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskUpdate {
    Title { title: String },
    Finalizing,
    PeerStats(PeerStats),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineMessage {
    Ready {
        version: u32,
    },
    Progress {
        progress: TransferProgress,
    },
    TaskUpdate {
        update: TaskUpdate,
    },
    Completed {
        /// Regular files in the workspace, using relative slash-separated paths.
        files: Vec<String>,
    },
    Inspected {
        source: SourceFiles,
    },
    Stopped,
    Failed {
        reason: FailureReason,
    },
}

/// Serialize one newline-terminated JSON message.
pub fn encode_message(message: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(invalid_frame());
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Read a complete message, or return `None` on a clean end of stream.
pub fn read_message<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.pop() != Some(b'\n') || bytes.len() > MAX_MESSAGE_BYTES {
        return Err(invalid_frame());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| invalid_frame())
}

pub fn write_message(writer: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
    writer.write_all(&encode_message(message)?)?;
    writer.flush()
}

fn invalid_frame() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid engine message")
}
