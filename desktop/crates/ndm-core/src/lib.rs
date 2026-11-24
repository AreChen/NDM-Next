mod cache;
mod directories;
pub use cache::CacheReport;
#[cfg(target_os = "linux")]
pub use directories::linux_config_directory;
pub use directories::{
    application_data_directory, default_cache_directory, resolve_download_directory,
    system_download_directory,
};
mod continuation;
mod dash;
mod diagnostics;
mod ftp;
mod hls;
mod http;
#[cfg(windows)]
mod installation;
mod instance;
mod media;
mod model;
pub mod plugin_engine;
mod plugin_transfer;
pub mod plugins;
mod scheduler;
mod secrets;
mod sftp;
mod speed;
mod store;
mod task_progress;

pub use dash::{
    DashAudio, DashCatalog, DashOptions, DashOutcome, DashRepresentation, download_dash,
    download_dash_with_media_tool, execute_dash, inspect_dash, inspect_dash_with_options,
};
pub use diagnostics::{
    DownloadFailure, HttpDiagnostics, HttpRangeMode, HttpStage, ProxyMode, ProxyRoute,
    TaskActivity, validate_user_agent,
};
pub use ftp::{FtpOptions, FtpSecurity, download_ftp, execute_ftp};
pub use hls::{
    HlsAudio, HlsCatalog, HlsOptions, HlsOutcome, HlsVariant, download_hls,
    download_hls_with_media_tool, execute_hls, inspect_hls, inspect_hls_with_options,
};
pub use http::{
    CHROME_USER_AGENT, DownloadProgress, HttpOptions, ProgressPhase, ProgressSnapshot,
    ProxyOptions, SegmentSnapshot, download_http, download_http_with_client, execute_http,
    http_filename_hint,
};
#[cfg(windows)]
pub use installation::shutdown_desktop_installation;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub use instance::desktop_is_running;
#[cfg(windows)]
pub use instance::windows_installer_active;
pub use instance::{
    DesktopUpdateGuard, desktop_installation_active, desktop_installation_namespace,
    desktop_instance_namespace, with_desktop_startup,
};
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub use instance::{request_desktop_shutdown, wait_for_desktop_ready, wake_desktop};
pub use model::{
    BatchLimit, DownloadLimits, NewTask, PluginOptions, Task, TaskId, TaskOptions, TaskPolicy,
    TaskState,
};
pub use ndm_plugin_sdk::PeerStats;
pub use scheduler::{Scheduler, SchedulerEvent, SchedulerOptions, SharingStatus};
pub use sftp::{SftpAuth, SftpOptions, download_sftp, execute_sftp};
pub use speed::SpeedMeter;
pub use store::{PendingSftpHost, Store, TaskCommand};
pub use task_progress::recover_task_progress;

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
    #[error("download SHA-256 does not match the source")]
    ChecksumMismatch,
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
    #[error("FTP server returned an inconsistent or changed file")]
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
    #[error("HLS media codec is unsupported")]
    HlsCodecUnsupported,
    #[error("HLS recovery data or media no longer matches")]
    HlsRecovery,
    #[error("DASH manifest, media, or output is invalid")]
    Dash,
    #[error("DRM-protected DASH media is unsupported")]
    DashDrmUnsupported,
    #[error("dynamic or live DASH media is unsupported")]
    DashDynamicUnsupported,
    #[error("this DASH manifest layout or media format is unsupported")]
    DashUnsupported,
    #[error("DASH recovery data or media no longer matches")]
    DashRecovery,
    #[error("trusted local media tools are unavailable")]
    MediaToolUnavailable,
    #[error("download engine plugin is unavailable")]
    PluginUnavailable,
    #[error("download engine plugin authentication failed")]
    PluginAuthentication,
    #[error("download engine plugin network error")]
    PluginNetwork,
    #[error("download engine plugin does not support this source")]
    PluginSource,
    #[error("download engine plugin could not save files")]
    PluginStorage,
    #[error("download engine plugin settings are invalid")]
    PluginOptions,
    #[error("download engine plugin could not finish processing the files")]
    PluginPostProcessing,
    #[error("download engine plugin failed")]
    PluginTransfer,
    #[error("download was paused or cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Encrypt data for the current user using the same protection as task credentials.
pub fn protect_user_data(data: &[u8]) -> Result<Vec<u8>> {
    secrets::protect(data)
}
/// Decrypt data previously protected for the current user.
pub fn unprotect_user_data(data: &[u8]) -> Result<Vec<u8>> {
    secrets::unprotect(data)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod unix_instance;
#[cfg(target_os = "macos")]
pub use unix_instance as macos_instance;
