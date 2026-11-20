//! Wire types and framing for NDM Next download engine workers.

mod manifest;
mod worker;

pub use manifest::*;
pub use worker::{Worker, serve};

use std::{
    collections::BTreeMap,
    io::{self, BufRead, Read, Write},
    num::NonZeroU32,
    path::PathBuf,
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const ENGINE_PROTOCOL_VERSION: u32 = 1;
pub const ENGINE_ARGUMENT: &str = "--ndm-engine";
pub const MEDIA_TOOLS_ENV: &str = "NDM_MEDIA_TOOLS";
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

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
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    Hello { version: u32 },
    Download { request: Box<DownloadRequest> },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    InvalidSource,
    Authentication,
    Network,
    Storage,
    Unsupported,
    Other,
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
    Completed {
        /// Regular files in the workspace, using relative slash-separated paths.
        files: Vec<String>,
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
