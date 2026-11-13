use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use percent_encoding::percent_decode_str;
use russh::{
    client,
    keys::{HashAlg, PublicKeyOrCertificate, key::PrivateKeyWithHashAlg, load_secret_key},
};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
use tokio_util::sync::CancellationToken;

use crate::{
    DownloadProgress, Error, NewTask, Result, Store, TaskId,
    http::{RateLimiter, publish_no_overwrite, safe_filename},
};

const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SftpAuth {
    Password(String),
    PrivateKey {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

impl fmt::Debug for SftpAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => f.write_str("Password(<redacted>)"),
            Self::PrivateKey { path, .. } => f
                .debug_struct("PrivateKey")
                .field("path", path)
                .field("passphrase", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpOptions {
    /// SHA256 fingerprint explicitly trusted for this host and port.
    pub trusted_fingerprint: Option<String>,
    /// Defaults to username and password in the encrypted task URL.
    pub auth: Option<SftpAuth>,
    pub max_bytes_per_second: Option<u64>,
}

struct HostKeyCheck {
    trusted: Option<String>,
    observed: Arc<Mutex<Option<String>>>,
}

impl client::Handler for HostKeyCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let accepted = self.trusted.as_ref() == Some(&fingerprint);
        if let Ok(mut observed) = self.observed.lock() {
            *observed = Some(fingerprint);
        }
        Ok(accepted)
    }
}

pub async fn execute_sftp(
    store: &Store,
    id: TaskId,
    options: &SftpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
) -> Result<PathBuf> {
    let task = store.get(id)?.ok_or(Error::NotFound)?;
    store.start(id)?;
    match download_sftp_with_progress(&task.request, id, options, cancel, progress.clone()).await {
        Ok(path) => {
            store.complete(id, &path, progress.snapshot().total.is_some())?;
            Ok(path)
        }
        Err(Error::Cancelled) => {
            store.pause(id)?;
            Err(Error::Cancelled)
        }
        Err(error) => {
            match &error {
                Error::SftpUnknownHost { fingerprint } => {
                    store.fail_sftp_host(id, fingerprint, None)?;
                    return Err(error);
                }
                Error::SftpHostKeyChanged { expected, observed } => {
                    store.fail_sftp_host(id, observed, Some(expected))?;
                    return Err(error);
                }
                _ => {}
            }
            let reason = match &error {
                Error::SftpAuthentication => "SFTP authentication failed",
                Error::InvalidSftpResponse => "SFTP server returned an inconsistent file",
                Error::Io(_) => "file or operating system error",
                _ => "SFTP connection or transfer failed",
            };
            store.fail(id, reason)?;
            Err(error)
        }
    }
}

pub async fn download_sftp(
    request: &NewTask,
    id: TaskId,
    options: &SftpOptions,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    download_sftp_with_progress(request, id, options, cancel, DownloadProgress::default()).await
}

async fn download_sftp_with_progress(
    request: &NewTask,
    id: TaskId,
    options: &SftpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
) -> Result<PathBuf> {
    let url = reqwest::Url::parse(&request.url).map_err(|_| Error::InvalidSftpAddress)?;
    if url.scheme() != "sftp" || url.query().is_some() || url.fragment().is_some() {
        return Err(Error::InvalidSftpAddress);
    }
    let host = url.host_str().ok_or(Error::InvalidSftpAddress)?;
    let port = url.port().unwrap_or(22);
    let username = decode_field(url.username())?;
    let password = decode_field(url.password().unwrap_or(""))?;
    let path = decode_field(url.path())?;
    if username.is_empty() || path.is_empty() || path == "/" {
        return Err(Error::InvalidSftpAddress);
    }
    let observed = Arc::new(Mutex::new(None));
    let handler = HostKeyCheck {
        trusted: options.trusted_fingerprint.clone(),
        observed: observed.clone(),
    };
    let setup = async {
        let session =
            client::connect(Arc::new(client::Config::default()), (host, port), handler).await;
        let mut session = match session {
            Ok(session) => session,
            Err(_) => {
                let observed = observed.lock().ok().and_then(|value| value.clone());
                return Err(match (options.trusted_fingerprint.clone(), observed) {
                    (None, Some(fingerprint)) => Error::SftpUnknownHost { fingerprint },
                    (Some(expected), Some(observed)) if expected != observed => {
                        Error::SftpHostKeyChanged { expected, observed }
                    }
                    _ => Error::SftpTransfer,
                });
            }
        };
        let result = match &options.auth {
            Some(SftpAuth::PrivateKey { path, passphrase }) => {
                let key = load_secret_key(path, passphrase.as_deref())
                    .map_err(|_| Error::SftpAuthentication)?;
                let hash = session
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|_| Error::SftpTransfer)?
                    .flatten();
                session
                    .authenticate_publickey(
                        &username,
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await
            }
            Some(SftpAuth::Password(value)) => {
                session.authenticate_password(&username, value).await
            }
            None => session.authenticate_password(&username, &password).await,
        }
        .map_err(|_| Error::SftpAuthentication)?;
        if !result.success() {
            return Err(Error::SftpAuthentication);
        }
        let channel = session
            .channel_open_session()
            .await
            .map_err(|_| Error::SftpTransfer)?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|_| Error::SftpTransfer)?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|_| Error::SftpTransfer)?;
        let metadata = sftp
            .metadata(path.clone())
            .await
            .map_err(|_| Error::SftpTransfer)?;
        if let (Some(expected), Some(actual)) = (request.expected_size, metadata.size)
            && expected != actual
        {
            return Err(Error::InvalidSftpResponse);
        }
        let file = sftp
            .open(path.clone())
            .await
            .map_err(|_| Error::SftpTransfer)?;
        Ok((session, sftp, file, metadata))
    };
    let (_session, sftp, mut input, metadata) = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        result = tokio::time::timeout(NETWORK_TIMEOUT, setup) => result.map_err(|_| Error::SftpTransfer)??,
    };
    let size = metadata.size.or(request.expected_size);
    progress.set_total(size);
    fs::create_dir_all(&request.destination).await?;
    let work_dir = request.destination.join(format!(".ndm-next-{id}"));
    fs::create_dir_all(&work_dir).await?;
    let temporary = work_dir.join("sftp.part");
    // SFTP metadata does not provide a strong content identity; restart on retry.
    let mut output = fs::File::create(&temporary).await?;
    let limiter = RateLimiter::new(options.max_bytes_per_second);
    let mut written = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                output.flush().await?;
                return Err(Error::Cancelled);
            }
            result = tokio::time::timeout(NETWORK_TIMEOUT, input.read(&mut buffer)) => {
                result.map_err(|_| Error::SftpTransfer)??
            }
        };
        if count == 0 {
            break;
        }
        limiter.wait(count, &cancel).await?;
        output.write_all(&buffer[..count]).await?;
        progress.add(count as u64);
        written = written
            .checked_add(count as u64)
            .ok_or(Error::InvalidSftpResponse)?;
        if size.is_some_and(|expected| written > expected) {
            return Err(Error::InvalidSftpResponse);
        }
    }
    if size.is_some_and(|expected| written != expected) {
        return Err(Error::InvalidSftpResponse);
    }
    let after = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        result = tokio::time::timeout(NETWORK_TIMEOUT, sftp.metadata(path.clone())) => {
            result.map_err(|_| Error::SftpTransfer)?.map_err(|_| Error::SftpTransfer)?
        }
    };
    if metadata.size != after.size || metadata.mtime != after.mtime {
        return Err(Error::InvalidSftpResponse);
    }
    output.flush().await?;
    output.sync_all().await?;
    drop(output);
    let filename = safe_filename(
        request
            .suggested_filename
            .as_deref()
            .or_else(|| path.rsplit('/').next())
            .unwrap_or("download"),
    );
    let target = publish_no_overwrite(&temporary, &request.destination, &filename).await?;
    let _ = fs::remove_file(&temporary).await;
    let _ = fs::remove_dir(&work_dir).await;
    Ok(target)
}

fn decode_field(field: &str) -> Result<String> {
    let value = percent_decode_str(field)
        .decode_utf8()
        .map_err(|_| Error::InvalidSftpAddress)?;
    if value.chars().any(|c| c == '\0' || c == '\r' || c == '\n') {
        return Err(Error::InvalidSftpAddress);
    }
    Ok(value.into_owned())
}
