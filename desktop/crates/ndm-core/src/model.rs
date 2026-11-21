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
    Plugin(PluginOptions),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginOptions {
    pub id: String,
    #[serde(default)]
    pub values: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<ndm_plugin_sdk::SourceSelection>,
}

impl fmt::Debug for PluginOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginOptions")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPolicy {
    pub batch: Option<BatchLimit>,
    pub download_threads: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchLimit {
    pub id: TaskId,
    pub active_tasks: u32,
}

impl TaskPolicy {
    pub(crate) fn valid(self) -> bool {
        self.batch
            .is_none_or(|batch| (1..=32).contains(&batch.active_tasks))
            && self
                .download_threads
                .is_none_or(|threads| (1..=256).contains(&threads))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadLimits {
    pub active_tasks: u32,
    pub download_threads: u32,
}

impl Default for DownloadLimits {
    fn default() -> Self {
        Self {
            active_tasks: 3,
            download_threads: 16,
        }
    }
}

impl DownloadLimits {
    pub(crate) fn valid(self) -> bool {
        (1..=32).contains(&self.active_tasks) && (1..=256).contains(&self.download_threads)
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Prepared,
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
}

impl TaskState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

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
    /// Persisted per task so changing the cache setting cannot discard resumable data.
    pub cache_directory: Option<PathBuf>,
    pub id: TaskId,
    pub request: NewTask,
    pub options: TaskOptions,
    pub policy: TaskPolicy,
    pub state: TaskState,
    /// Title reported by an engine after resolving the source.
    pub engine_title: Option<String>,
    pub output_path: Option<PathBuf>,
    pub error: Option<String>,
    pub size_verified: Option<bool>,
    pub progress: Option<crate::ProgressSnapshot>,
}

impl Task {
    pub fn work_directory(&self) -> PathBuf {
        self.cache_directory
            .as_deref()
            .unwrap_or(&self.request.destination)
            .join(format!(".ndm-next-{}", self.id))
    }
}
