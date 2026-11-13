mod http;
mod model;
mod secrets;
mod store;

pub use http::{DownloadProgress, HttpOptions, ProgressSnapshot, download_http, execute_http};
pub use model::{NewTask, Task, TaskId, TaskState};
pub use store::Store;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("task database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("invalid task data: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("operating system error: {0}")]
    Io(#[from] std::io::Error),
    #[error("task does not exist")]
    NotFound,
    #[error("task identifier is already in use")]
    IdConflict,
    #[error("task cannot transition from its current state")]
    InvalidState,
    #[error("task database contains an invalid state")]
    InvalidStoredState,
    #[error("task database was created by a newer version")]
    UnsupportedSchema,
    #[error("credential protection is unavailable on this platform")]
    UnsupportedPlatform,
    #[error("HTTP request failed: {0}")]
    Network(#[source] reqwest::Error),
    #[error("HTTP server returned status {0}")]
    HttpStatus(u16),
    #[error("HTTP server returned an invalid or inconsistent response")]
    InvalidHttpResponse,
    #[error("download request contains an invalid or reserved header")]
    InvalidHeader,
    #[error("download was paused or cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;
