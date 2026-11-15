use std::{path::PathBuf, time::Duration};

use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use suppaftp::{
    FtpError, Status,
    async_native_tls::{Certificate, TlsConnector},
    tokio::{AsyncNativeTlsConnector, AsyncNativeTlsFtpStream},
    types::FileType,
};
use tokio::{fs, io::AsyncReadExt};
use tokio_util::sync::CancellationToken;

use crate::{
    DownloadProgress, Error, NewTask, Result, Store, TaskId,
    continuation::{PartialFile, source_binding},
    http::{RateLimiter, publish_download, safe_filename},
};

const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FtpSecurity {
    /// Plain FTP for ftp:// and explicit FTPS for ftps://.
    #[default]
    Auto,
    Plain,
    ExplicitTls,
    ImplicitTls,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FtpOptions {
    pub security: FtpSecurity,
    pub max_bytes_per_second: Option<u64>,
    /// An additional explicitly trusted CA, in DER format.
    pub additional_ca_der: Option<Vec<u8>>,
}

pub async fn execute_ftp(
    store: &Store,
    id: TaskId,
    options: &FtpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
) -> Result<PathBuf> {
    let task = store.get(id)?.ok_or(Error::NotFound)?;
    store.start(id)?;
    match download_ftp_with_progress(
        &task.request,
        id,
        options,
        cancel,
        progress.clone(),
        Some(&task.work_directory()),
    )
    .await
    {
        Ok(path) => {
            store.complete(id, &path, progress.snapshot().total.is_some())?;
            Ok(path)
        }
        Err(Error::Cancelled) => {
            store.pause(id)?;
            Err(Error::Cancelled)
        }
        Err(error) => {
            let reason = match error {
                Error::FtpAuthentication => "FTP authentication failed",
                Error::InvalidFtpResponse => "FTP file changed or failed integrity validation",
                Error::Io(_) => "file or operating system error",
                _ => "FTP connection or transfer failed",
            };
            store.fail(id, reason)?;
            Err(error)
        }
    }
}

pub async fn download_ftp(
    request: &NewTask,
    id: TaskId,
    options: &FtpOptions,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    download_ftp_with_progress(
        request,
        id,
        options,
        cancel,
        DownloadProgress::default(),
        None,
    )
    .await
}

async fn download_ftp_with_progress(
    request: &NewTask,
    id: TaskId,
    options: &FtpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
    work_directory: Option<&std::path::Path>,
) -> Result<PathBuf> {
    let url = reqwest::Url::parse(&request.url).map_err(|_| Error::InvalidFtpAddress)?;
    if !matches!(url.scheme(), "ftp" | "ftps") || url.query().is_some() || url.fragment().is_some()
    {
        return Err(Error::InvalidFtpAddress);
    }
    let host = url.host_str().ok_or(Error::InvalidFtpAddress)?;
    let security = match options.security {
        FtpSecurity::Auto if url.scheme() == "ftps" => FtpSecurity::ExplicitTls,
        FtpSecurity::Auto => FtpSecurity::Plain,
        other => other,
    };
    if url.scheme() == "ftps" && security == FtpSecurity::Plain {
        return Err(Error::InvalidFtpAddress);
    }
    let port = url
        .port()
        .unwrap_or(if security == FtpSecurity::ImplicitTls {
            990
        } else {
            21
        });
    let path = decode_field(url.path())?;
    let username = if url.username().is_empty() {
        "anonymous".to_owned()
    } else {
        decode_field(url.username())?
    };
    let password = decode_field(url.password().unwrap_or("anonymous@"))?;
    if path == "/" || path.is_empty() {
        return Err(Error::InvalidFtpAddress);
    }

    let setup = async {
        let connector = || -> Result<AsyncNativeTlsConnector> {
            let mut tls = TlsConnector::new();
            if let Some(der) = &options.additional_ca_der {
                let cert = Certificate::from_der(der).map_err(|_| Error::FtpTransfer)?;
                tls = tls.add_root_certificate(cert);
            }
            Ok(AsyncNativeTlsConnector::from(tls))
        };
        let mut ftp = match security {
            FtpSecurity::Plain => AsyncNativeTlsFtpStream::connect((host, port)).await,
            FtpSecurity::ExplicitTls => {
                let connection = AsyncNativeTlsFtpStream::connect((host, port))
                    .await
                    .map_err(|_| Error::FtpTransfer)?;
                connection.into_secure(connector()?, host).await
            }
            FtpSecurity::ImplicitTls => {
                AsyncNativeTlsFtpStream::connect_secure_implicit((host, port), connector()?, host)
                    .await
            }
            FtpSecurity::Auto => unreachable!(),
        }
        .map_err(|_| Error::FtpTransfer)?;
        // A server-provided PASV address must not redirect the data socket to a third party.
        ftp.set_passive_nat_workaround(true);
        ftp.login(&username, &password)
            .await
            .map_err(|_| Error::FtpAuthentication)?;
        if security == FtpSecurity::ImplicitTls {
            ftp.custom_command("PBSZ 0", &[Status::CommandOk])
                .await
                .map_err(|_| Error::FtpTransfer)?;
            ftp.custom_command("PROT P", &[Status::CommandOk])
                .await
                .map_err(|_| Error::FtpTransfer)?;
        }
        ftp.transfer_type(FileType::Binary)
            .await
            .map_err(|_| Error::FtpTransfer)?;
        let size = ftp.size(&path).await.ok().map(|size| size as u64);
        if let (Some(expected), Some(actual)) = (request.expected_size, size)
            && expected != actual
        {
            return Err(Error::InvalidFtpResponse);
        }
        let size = size.or(request.expected_size);
        let modified = ftp.mdtm(&path).await.ok();
        let sha256 = remote_sha256(&mut ftp, &path, size).await;
        Ok((ftp, size, modified, sha256))
    };
    let (mut ftp, size, modified, sha256) = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        result = tokio::time::timeout(NETWORK_TIMEOUT, setup) => result.map_err(|_| Error::FtpTransfer)??,
    };
    progress.set_total(size);
    fs::create_dir_all(&request.destination).await?;
    let work_dir = work_directory
        .map(std::path::Path::to_owned)
        .unwrap_or_else(|| request.destination.join(format!(".ndm-next-{id}")));
    fs::create_dir_all(&work_dir).await?;
    crate::cache::validate_directory(&work_dir)?;
    let temporary = work_dir.join("ftp.part");
    let binding = source_binding(&(&request.url, security, &options.additional_ca_der))?;
    let mut partial =
        PartialFile::open(&work_dir, &temporary, binding, size, sha256, &cancel).await?;
    let limiter = RateLimiter::new(options.max_bytes_per_second);
    let transfer = async {
        let mut offset = partial.resume_offset();
        if offset > 0 {
            ftp.resume_transfer(usize::try_from(offset).map_err(|_| Error::InvalidFtpResponse)?)
                .await.map_err(|_| Error::FtpTransfer)?;
        }
        let open = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            result = tokio::time::timeout(NETWORK_TIMEOUT, ftp.retr_as_stream(&path)) => {
                result.map_err(|_| Error::FtpTransfer)?
            }
        };
        let mut data = match open {
            Ok(data) => data,
            Err(FtpError::UnexpectedResponse(response)) if offset > 0 && matches!(response.status.code(), 500 | 501 | 502 | 504 | 550) => {
                // REST is optional. Keep the saved file, but re-read/compare its prefix.
                partial.require_prefix_validation();
                offset = 0;
                ftp.resume_transfer(0).await.map_err(|_| Error::FtpTransfer)?;
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(Error::Cancelled),
                    result = tokio::time::timeout(NETWORK_TIMEOUT, ftp.retr_as_stream(&path)) => {
                        result.map_err(|_| Error::FtpTransfer)?.map_err(|_| Error::FtpTransfer)?
                    }
                }
            }
            Err(_) => return Err(Error::FtpTransfer),
        };
        progress.add_existing(offset);
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                result = tokio::time::timeout(NETWORK_TIMEOUT, data.read(&mut buffer)) => {
                    result.map_err(|_| Error::FtpTransfer)??
                }
            };
            if count == 0 { break; }
            progress.received(count as u64);
            if size.is_some_and(|expected| partial.received().saturating_add(count as u64) > expected) {
                return Err(Error::InvalidFtpResponse);
            }
            limiter.wait(count, &cancel).await?;
            partial.consume(&buffer[..count]).await?;
            progress.add(count as u64);
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            result = tokio::time::timeout(NETWORK_TIMEOUT, data.finish()) => {
                result.map_err(|_| Error::FtpTransfer)?.map_err(|_| Error::FtpTransfer)?;
            }
        }
        if size.is_some_and(|expected| partial.received() != expected) {
            return Err(Error::InvalidFtpResponse);
        }
        let (after_modified, after_sha256) = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            result = tokio::time::timeout(NETWORK_TIMEOUT, async {
                (ftp.mdtm(&path).await.ok(), if sha256.is_some() { remote_sha256(&mut ftp, &path, size).await } else { None })
            }) => result.map_err(|_| Error::FtpTransfer)?,
        };
        if modified != after_modified || sha256 != after_sha256 || !partial.matches_remote_hash() {
            return Err(Error::InvalidFtpResponse);
        }
        partial.finish().await
    }.await;
    if transfer.is_err() {
        partial.save_checkpoint().await?;
    }
    if matches!(transfer, Err(Error::InvalidFtpResponse)) {
        // A digest mismatch or inconsistent transfer must never authorize a future REST.
        partial.invalidate().await?;
    }
    transfer?;
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    drop(partial);
    let filename = safe_filename(
        request
            .suggested_filename
            .as_deref()
            .or_else(|| path.rsplit('/').next())
            .unwrap_or("download"),
    );
    let target = publish_download(
        &temporary,
        &request.destination,
        &filename,
        &progress,
        &cancel,
    )
    .await?;
    let _ = fs::remove_file(&temporary).await;
    PartialFile::cleanup(&work_dir).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), ftp.quit()).await;
    Ok(target)
}

/// Optional HASH extension, deliberately restricted to an unambiguous full-file SHA-256.
/// Other hashes/ranges/legacy response formats fall back to full-prefix verification.
async fn remote_sha256(
    ftp: &mut AsyncNativeTlsFtpStream,
    path: &str,
    size: Option<u64>,
) -> Option<[u8; 32]> {
    let size = size.filter(|size| *size > 0)?;
    ftp.custom_command("OPTS HASH SHA-256", &[Status::CommandOk])
        .await
        .ok()?;
    let response = ftp
        .custom_command(format!("HASH {path}"), &[Status::File])
        .await
        .ok()?
        .as_string()
        .ok()?;
    let line = response.lines().last()?.strip_prefix("213 ")?;
    let mut fields = line.splitn(4, ' ');
    if !fields.next()?.eq_ignore_ascii_case("SHA-256") {
        return None;
    }
    let (start, end) = fields.next()?.split_once('-')?;
    if start.parse::<u64>().ok()? != 0 || end.parse::<u64>().ok()?.checked_add(1)? != size {
        return None;
    }
    let hex = fields.next()?;
    if hex.len() != 64 || fields.next()? != path {
        return None;
    }
    let mut digest = [0; 32];
    for (byte, pair) in digest.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(digest)
}

fn decode_field(field: &str) -> Result<String> {
    let value = percent_decode_str(field)
        .decode_utf8()
        .map_err(|_| Error::InvalidFtpAddress)?;
    if value.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
        return Err(Error::InvalidFtpAddress);
    }
    Ok(value.into_owned())
}
