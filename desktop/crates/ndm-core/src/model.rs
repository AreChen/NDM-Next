use std::{collections::BTreeMap, fmt, path::PathBuf};

use crate::{DashOptions, FtpOptions, HlsOptions, HttpOptions, SftpOptions};
use serde::{Deserialize, Serialize};

pub type TaskId = uuid::Uuid;

/// Protocol-specific choices are encrypted together with other task secrets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskOptions {
    #[default]
    Auto,
    /// HTTP with the current application defaults, independent of the URL suffix.
    AutoHttp,
    Http(HttpOptions),
    Ftp(FtpOptions),
    Sftp(SftpOptions),
    Hls(HlsOptions),
    Dash(DashOptions),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    pub url: String,
    pub destination: PathBuf,
    pub suggested_filename: Option<String>,
    pub source_url: Option<String>,
    pub expected_size: Option<u64>,
    pub headers: BTreeMap<String, String>,
}

// Download URLs and headers may contain bearer tokens.
impl fmt::Debug for NewTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewTask")
            .field("url", &"<redacted>")
            .field("destination", &self.destination)
            .field("suggested_filename", &self.suggested_filename)
            .field(
                "source_url",
                &self.source_url.as_ref().map(|_| "<redacted>"),
            )
            .field("expected_size", &self.expected_size)
            .field("headers", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Prepared,
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
}

impl TaskState {
    pub(crate) fn from_database(value: &str) -> crate::Result<Self> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(crate::Error::InvalidStoredState),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Task {
    pub id: TaskId,
    pub request: NewTask,
    pub options: TaskOptions,
    pub state: TaskState,
    pub output_path: Option<PathBuf>,
    pub error: Option<String>,
    pub size_verified: Option<bool>,
    pub progress: Option<crate::ProgressSnapshot>,
}
