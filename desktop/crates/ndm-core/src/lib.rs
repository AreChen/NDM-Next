mod ftp;
mod hls;
mod http;
mod model;
mod scheduler;
mod secrets;
mod sftp;
mod store;

pub use ftp::{FtpOptions, FtpSecurity, download_ftp, execute_ftp};
pub use hls::{
    HlsAudio, HlsCatalog, HlsOptions, HlsOutcome, HlsVariant, download_hls, execute_hls,
    inspect_hls, inspect_hls_with_options,
};
pub use http::{
    DownloadProgress, HttpOptions, ProgressSnapshot, ProxyOptions, download_http, execute_http,
};
pub use model::{NewTask, Task, TaskId, TaskOptions, TaskState};
pub use scheduler::{Scheduler, SchedulerEvent, SchedulerOptions};
pub use sftp::{SftpAuth, SftpOptions, download_sftp, execute_sftp};
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
    #[error("replacement download address is invalid")]
    InvalidDownloadAddress,
    #[error("proxy configuration is invalid")]
    InvalidProxy,
    #[error("invalid FTP address or unsafe FTP command")]
    InvalidFtpAddress,
    #[error("FTP server connection or transfer failed")]
    FtpTransfer,
    #[error("FTP authentication failed")]
    FtpAuthentication,
    #[error("FTP server returned an inconsistent file size")]
    InvalidFtpResponse,
    #[error("invalid SFTP address")]
    InvalidSftpAddress,
    #[error("SFTP host key is not trusted: {fingerprint}")]
    SftpUnknownHost { fingerprint: String },
    #[error("SFTP host key changed: expected {expected}, received {observed}")]
    SftpHostKeyChanged { expected: String, observed: String },
    #[error("SFTP authentication failed")]
    SftpAuthentication,
    #[error("SFTP server connection or transfer failed")]
    SftpTransfer,
    #[error("SFTP server returned an inconsistent file")]
    InvalidSftpResponse,
    #[error("HLS playlist, media, or output is invalid or unsupported")]
    Hls,
    #[error("download was paused or cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Encrypt data for the current Windows user using the same protection as task credentials.
pub fn protect_user_data(data: &[u8]) -> Result<Vec<u8>> {
    secrets::protect(data)
}
/// Decrypt data previously protected for the current Windows user.
pub fn unprotect_user_data(data: &[u8]) -> Result<Vec<u8>> {
    secrets::unprotect(data)
}
