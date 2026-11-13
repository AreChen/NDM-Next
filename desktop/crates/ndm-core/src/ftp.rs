use std::{path::PathBuf, time::Duration};

use percent_encoding::percent_decode_str;
use suppaftp::{
    Status,
    async_native_tls::{Certificate, TlsConnector},
    tokio::{AsyncNativeTlsConnector, AsyncNativeTlsFtpStream},
    types::FileType,
};
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FtpSecurity {
    /// Plain FTP for ftp:// and explicit FTPS for ftps://.
    #[default]
    Auto,
    Plain,
    ExplicitTls,
    ImplicitTls,
}

#[derive(Clone, Debug, Default)]
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
    match download_ftp_with_progress(&task.request, id, options, cancel, progress.clone()).await {
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
                Error::InvalidFtpResponse => "FTP server returned an inconsistent file size",
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
    download_ftp_with_progress(request, id, options, cancel, DownloadProgress::default()).await
}

async fn download_ftp_with_progress(
    request: &NewTask,
    id: TaskId,
    options: &FtpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
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
        Ok((ftp, size.or(request.expected_size)))
    };
    let (mut ftp, size) = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        result = tokio::time::timeout(NETWORK_TIMEOUT, setup) => result.map_err(|_| Error::FtpTransfer)??,
    };
    progress.set_total(size);
    fs::create_dir_all(&request.destination).await?;
    let work_dir = request.destination.join(format!(".ndm-next-{id}"));
    fs::create_dir_all(&work_dir).await?;
    let temporary = work_dir.join("ftp.part");
    // FTP SIZE and timestamps are not strong enough to safely identify an old file.
    // Restart from zero unless a future protocol-specific validator proves identity.
    let mut output = fs::File::create(&temporary).await?;
    let mut data = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        result = tokio::time::timeout(NETWORK_TIMEOUT, ftp.retr_as_stream(&path)) => {
            result.map_err(|_| Error::FtpTransfer)?.map_err(|_| Error::FtpTransfer)?
        }
    };
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
            result = tokio::time::timeout(NETWORK_TIMEOUT, data.read(&mut buffer)) => {
                result.map_err(|_| Error::FtpTransfer)??
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
            .ok_or(Error::InvalidFtpResponse)?;
        if size.is_some_and(|expected| written > expected) {
            return Err(Error::InvalidFtpResponse);
        }
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        result = tokio::time::timeout(NETWORK_TIMEOUT, data.finish()) => {
            result.map_err(|_| Error::FtpTransfer)?.map_err(|_| Error::FtpTransfer)?;
        }
    }
    if size.is_some_and(|expected| written != expected) {
        return Err(Error::InvalidFtpResponse);
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
    let _ = tokio::time::timeout(Duration::from_secs(1), ftp.quit()).await;
    Ok(target)
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
