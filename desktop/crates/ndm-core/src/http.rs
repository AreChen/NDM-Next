use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use reqwest::{
    Client, Response, StatusCode,
    header::{self, HeaderMap, HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use tokio::{fs, io::AsyncWriteExt, sync::Mutex, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::{Error, NewTask, Result, Store, TaskId, secrets};

const MAX_CONNECTIONS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpOptions {
    pub max_connections: usize,
    pub max_bytes_per_second: Option<u64>,
}

#[derive(Default)]
struct ProgressState {
    downloaded: AtomicU64,
    total: AtomicU64,
    total_known: AtomicBool,
}

#[derive(Clone, Default)]
pub struct DownloadProgress(Arc<ProgressState>);

pub struct ProgressSnapshot {
    pub downloaded: u64,
    pub total: Option<u64>,
}

impl DownloadProgress {
    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            downloaded: self.0.downloaded.load(Ordering::Relaxed),
            total: self
                .0
                .total_known
                .load(Ordering::Acquire)
                .then(|| self.0.total.load(Ordering::Relaxed)),
        }
    }

    pub(crate) fn set_total(&self, total: Option<u64>) {
        if let Some(total) = total {
            self.0.total.store(total, Ordering::Relaxed);
            self.0.total_known.store(true, Ordering::Release);
        } else {
            self.0.total_known.store(false, Ordering::Release);
        }
    }

    pub(crate) fn add(&self, bytes: u64) {
        self.0.downloaded.fetch_add(bytes, Ordering::Relaxed);
    }
}

pub async fn execute_http(
    store: &Store,
    id: TaskId,
    options: &HttpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
) -> Result<PathBuf> {
    let task = store.get(id)?.ok_or(Error::NotFound)?;
    store.start(id)?;
    match download_http_with_progress(&task.request, id, options, cancel, progress.clone()).await {
        Ok(path) => {
            let size_verified =
                progress.snapshot().total.is_some() || task.request.expected_size.is_some();
            store.complete(id, &path, size_verified)?;
            Ok(path)
        }
        Err(Error::Cancelled) => {
            store.pause(id)?;
            Err(Error::Cancelled)
        }
        Err(error) => {
            let reason = match error {
                Error::HttpStatus(_) => "server rejected the download",
                Error::InvalidHttpResponse => "server returned an inconsistent response",
                Error::Network(_) => "network error",
                Error::Io(_) => "file or operating system error",
                _ => "download failed",
            };
            store.fail(id, reason)?;
            Err(error)
        }
    }
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            max_connections: 4,
            max_bytes_per_second: None,
        }
    }
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
struct ResumeManifest {
    validator: String,
    resource_url: String,
    total: u64,
    connections: usize,
}

pub async fn download_http(
    request: &NewTask,
    id: TaskId,
    options: &HttpOptions,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    download_http_with_progress(request, id, options, cancel, DownloadProgress::default()).await
}

async fn download_http_with_progress(
    request: &NewTask,
    id: TaskId,
    options: &HttpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
) -> Result<PathBuf> {
    progress.0.downloaded.store(0, Ordering::Relaxed);
    progress.set_total(None);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .build()
        .map_err(network)?;
    let headers = request_headers(request)?;
    fs::create_dir_all(&request.destination).await?;
    let work_dir = request.destination.join(format!(".ndm-next-{id}"));
    fs::create_dir_all(&work_dir).await?;
    let limiter = Arc::new(RateLimiter {
        bytes_per_second: options.max_bytes_per_second.filter(|v| *v > 0),
        next: Mutex::new(Instant::now()),
    });

    let probe = send(
        &client,
        client
            .get(&request.url)
            .headers(headers.clone())
            .header(header::RANGE, "bytes=0-0"),
        &cancel,
    )
    .await?;
    let ranged = if probe.status() == StatusCode::PARTIAL_CONTENT {
        probe
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range)
            .filter(|&(start, end, total)| start == 0 && end == 0 && total > 0)
            .and_then(|(_, _, total)| {
                strong_etag(probe.headers())
                    .map(|etag| (total, etag, probe.url().as_str().to_owned()))
            })
    } else {
        None
    };

    let assembled = work_dir.join("assembled.part");
    if let Some((total, validator, resource_url)) = ranged {
        drop(probe);
        progress.set_total(Some(total));
        let connections = options
            .max_connections
            .clamp(1, MAX_CONNECTIONS)
            .min(usize::try_from(total).unwrap_or(usize::MAX));
        let manifest = ResumeManifest {
            validator,
            resource_url,
            total,
            connections,
        };
        let manifest_path = work_dir.join("resume.bin");
        let existing = std::fs::read(&manifest_path)
            .ok()
            .and_then(|bytes| secrets::unprotect(&bytes).ok())
            .and_then(|bytes| serde_json::from_slice::<ResumeManifest>(&bytes).ok());
        if existing.as_ref() != Some(&manifest) {
            clear_parts(&work_dir).await?;
            let protected = secrets::protect(&serde_json::to_vec(&manifest)?)?;
            let mut file = fs::File::create(&manifest_path).await?;
            file.write_all(&protected).await?;
            file.sync_all().await?;
        }
        let segment_cancel = cancel.child_token();
        let mut transfers = FuturesUnordered::new();
        for index in 0..connections {
            let start = ((total as u128 * index as u128) / connections as u128) as u64;
            let end = ((total as u128 * (index + 1) as u128) / connections as u128) as u64 - 1;
            let part = work_dir.join(format!("segment-{index}.part"));
            transfers.push(fetch_segment(
                client.clone(),
                request.url.clone(),
                headers.clone(),
                part,
                start,
                end,
                total,
                manifest.validator.clone(),
                manifest.resource_url.clone(),
                limiter.clone(),
                segment_cancel.clone(),
                progress.clone(),
            ));
        }
        let mut first_error = None;
        while let Some(result) = transfers.next().await {
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
                segment_cancel.cancel();
            }
        }
        match first_error {
            Some(Error::InvalidHttpResponse) if !cancel.is_cancelled() => {
                progress.0.downloaded.store(0, Ordering::Relaxed);
                let response =
                    send(&client, client.get(&request.url).headers(headers), &cancel).await?;
                download_full(response, &assembled, &limiter, &cancel, &progress).await?;
            }
            Some(error) => return Err(error),
            None => {
                let mut merged = fs::File::create(&assembled).await?;
                for index in 0..connections {
                    let mut part =
                        fs::File::open(work_dir.join(format!("segment-{index}.part"))).await?;
                    tokio::io::copy(&mut part, &mut merged).await?;
                }
                merged.flush().await?;
                if merged.metadata().await?.len() != total {
                    return Err(Error::InvalidHttpResponse);
                }
                merged.sync_all().await?;
            }
        }
    } else {
        let response = if probe.status() == StatusCode::OK {
            probe
        } else if matches!(
            probe.status(),
            StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE
        ) {
            drop(probe);
            send(&client, client.get(&request.url).headers(headers), &cancel).await?
        } else {
            return Err(Error::HttpStatus(probe.status().as_u16()));
        };
        download_full(response, &assembled, &limiter, &cancel, &progress).await?;
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if let Some(size) = request.expected_size
        && fs::metadata(&assembled).await?.len() != size
    {
        return Err(Error::InvalidHttpResponse);
    }

    let filename = safe_filename(request.suggested_filename.as_deref().unwrap_or("download"));
    let target = publish_no_overwrite(&assembled, &request.destination, &filename).await?;
    // The target is already published; cleanup failures must not turn a
    // successful transfer into a failed task and cause a duplicate retry.
    let _ = clear_parts(&work_dir).await;
    let _ = fs::remove_dir(&work_dir).await;
    Ok(target)
}

async fn download_full(
    response: Response,
    assembled: &Path,
    limiter: &RateLimiter,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
) -> Result<()> {
    if response.status() != StatusCode::OK {
        return Err(Error::HttpStatus(response.status().as_u16()));
    }
    let expected = response.content_length();
    progress.set_total(expected);
    let mut file = fs::File::create(assembled).await?;
    let result = write_response(response, &mut file, limiter, cancel, progress).await;
    file.flush().await?;
    let written = result?;
    if expected.is_some_and(|expected| expected != written) {
        return Err(Error::InvalidHttpResponse);
    }
    file.sync_all().await?;
    Ok(())
}

pub(crate) fn request_headers(request: &NewTask) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let mut referer = request.source_url.as_deref();
    for (key, value) in &request.headers {
        let name = HeaderName::from_bytes(key.as_bytes()).map_err(|_| Error::InvalidHeader)?;
        if matches!(
            name.as_str(),
            "range"
                | "if-range"
                | "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "proxy-authorization"
                | "upgrade"
        ) {
            return Err(Error::InvalidHeader);
        }
        if name == header::REFERER {
            referer = Some(value);
            continue;
        }
        let value = HeaderValue::from_str(value).map_err(|_| Error::InvalidHeader)?;
        headers.insert(name, value);
    }
    if let Some(source) = referer {
        let mut source = reqwest::Url::parse(source).map_err(|_| Error::InvalidHeader)?;
        let destination = reqwest::Url::parse(&request.url).map_err(|_| Error::InvalidHeader)?;
        if matches!(source.scheme(), "http" | "https")
            && !(source.scheme() == "https" && destination.scheme() == "http")
        {
            source.set_username("").map_err(|_| Error::InvalidHeader)?;
            source
                .set_password(None)
                .map_err(|_| Error::InvalidHeader)?;
            source.set_fragment(None);
            if source.origin() != destination.origin() {
                source.set_path("/");
                source.set_query(None);
            }
            headers.insert(
                header::REFERER,
                HeaderValue::from_str(source.as_str()).map_err(|_| Error::InvalidHeader)?,
            );
        }
    }
    Ok(headers)
}

pub(crate) async fn send(
    client: &Client,
    builder: reqwest::RequestBuilder,
    cancel: &CancellationToken,
) -> Result<Response> {
    let mut request = builder.build().map_err(network)?;
    for _ in 0..=10 {
        let previous = request.url().clone();
        let mut next_request = request.try_clone().ok_or(Error::InvalidHttpResponse)?;
        let response = send_with_retries(client, &request, cancel).await?;
        if !matches!(
            response.status(),
            StatusCode::MOVED_PERMANENTLY
                | StatusCode::FOUND
                | StatusCode::SEE_OTHER
                | StatusCode::TEMPORARY_REDIRECT
                | StatusCode::PERMANENT_REDIRECT
        ) {
            return Ok(response);
        }
        let Some(location) = response.headers().get(header::LOCATION) else {
            return Ok(response);
        };
        let location = location.to_str().map_err(|_| Error::InvalidHttpResponse)?;
        let next = previous
            .join(location)
            .map_err(|_| Error::InvalidHttpResponse)?;
        if !matches!(next.scheme(), "http" | "https") {
            return Err(Error::InvalidHttpResponse);
        }
        if next.origin() != previous.origin() {
            let mut safe = HeaderMap::new();
            for name in [header::RANGE, header::IF_RANGE] {
                if let Some(value) = next_request.headers().get(&name) {
                    safe.insert(name, value.clone());
                }
            }
            *next_request.headers_mut() = safe;
        }
        *next_request.url_mut() = next;
        request = next_request;
    }
    Err(Error::InvalidHttpResponse)
}

async fn send_with_retries(
    client: &Client,
    request: &reqwest::Request,
    cancel: &CancellationToken,
) -> Result<Response> {
    for attempt in 0..3 {
        let outgoing = request.try_clone().ok_or(Error::InvalidHttpResponse)?;
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            response = client.execute(outgoing) => response,
        };
        let retry = match response {
            Ok(response)
                if matches!(response.status().as_u16(), 429 | 500 | 502 | 503 | 504)
                    && attempt < 2 =>
            {
                true
            }
            Ok(response) => return Ok(response),
            Err(_) if attempt < 2 => true,
            Err(error) => return Err(network(error)),
        };
        if retry {
            let delay = Duration::from_millis(200 * (1 << attempt));
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
    Err(Error::InvalidHttpResponse)
}

fn network(error: reqwest::Error) -> Error {
    Error::Network(error.without_url())
}

fn strong_etag(headers: &HeaderMap) -> Option<String> {
    let etag = headers.get(header::ETAG)?.to_str().ok()?;
    (etag.starts_with('"') && etag.ends_with('"') && etag.len() >= 2).then(|| etag.to_owned())
}

pub(crate) fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (start.parse().ok()?, end.parse().ok()?, total.parse().ok()?);
    (start <= end && end < total).then_some((start, end, total))
}

#[allow(clippy::too_many_arguments)]
async fn fetch_segment(
    client: Client,
    url: String,
    headers: HeaderMap,
    part: PathBuf,
    start: u64,
    end: u64,
    total: u64,
    validator: String,
    resource_url: String,
    limiter: Arc<RateLimiter>,
    cancel: CancellationToken,
    progress: DownloadProgress,
) -> Result<()> {
    let length = end - start + 1;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&part)
        .await?;
    let mut present = file.metadata().await?.len();
    if present > length {
        file.set_len(0).await?;
        present = 0;
    }
    progress.add(present);
    for attempt in 0..3 {
        if present == length {
            file.sync_data().await?;
            return Ok(());
        }
        let range_start = start + present;
        let response = send(
            &client,
            client
                .get(&url)
                .headers(headers.clone())
                .header(header::RANGE, format!("bytes={range_start}-{end}"))
                .header(header::IF_RANGE, &validator),
            &cancel,
        )
        .await?;
        if !matches!(
            response.status(),
            StatusCode::PARTIAL_CONTENT | StatusCode::OK | StatusCode::RANGE_NOT_SATISFIABLE
        ) {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        if response.status() != StatusCode::PARTIAL_CONTENT
            || response.url().as_str() != resource_url
            || response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range)
                != Some((range_start, end, total))
            || strong_etag(response.headers()).as_deref() != Some(validator.as_str())
        {
            return Err(Error::InvalidHttpResponse);
        }
        let result = write_response(response, &mut file, &limiter, &cancel, &progress).await;
        file.flush().await?;
        match result {
            Ok(written) if written == length - present => {
                file.sync_data().await?;
                return Ok(());
            }
            Ok(_) => return Err(Error::InvalidHttpResponse),
            Err(Error::Network(_)) if attempt < 2 => {
                let next_present = file.metadata().await?.len();
                if next_present < present || next_present > length {
                    return Err(Error::InvalidHttpResponse);
                }
                present = next_present;
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(Error::Cancelled),
                    _ = tokio::time::sleep(Duration::from_millis(200 * (1 << attempt))) => {}
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(Error::InvalidHttpResponse)
}

async fn write_response(
    response: Response,
    file: &mut fs::File,
    limiter: &RateLimiter,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
) -> Result<u64> {
    let mut body = response.bytes_stream();
    let mut total = 0u64;
    loop {
        let chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                file.flush().await?;
                return Err(Error::Cancelled);
            }
            chunk = body.next() => chunk,
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(network)?;
        limiter.wait(chunk.len(), cancel).await?;
        file.write_all(&chunk).await?;
        progress.add(chunk.len() as u64);
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or(Error::InvalidHttpResponse)?;
    }
    Ok(total)
}

pub(crate) struct RateLimiter {
    bytes_per_second: Option<u64>,
    next: Mutex<Instant>,
}

impl RateLimiter {
    pub(crate) fn new(bytes_per_second: Option<u64>) -> Self {
        Self {
            bytes_per_second: bytes_per_second.filter(|value| *value > 0),
            next: Mutex::new(Instant::now()),
        }
    }

    pub(crate) async fn wait(&self, bytes: usize, cancel: &CancellationToken) -> Result<()> {
        let Some(rate) = self.bytes_per_second else {
            return Ok(());
        };
        let duration = Duration::from_secs_f64(bytes as f64 / rate as f64);
        let mut next = self.next.lock().await;
        let ready = (*next).max(Instant::now()) + duration;
        *next = ready;
        drop(next);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(Error::Cancelled),
            _ = tokio::time::sleep_until(ready) => Ok(()),
        }
    }
}

pub(crate) fn safe_filename(suggested: &str) -> String {
    let name = Path::new(suggested)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let name: String = name
        .chars()
        .map(|ch| {
            if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let name = name.trim_end_matches([' ', '.']);
    if name.is_empty() {
        "download".into()
    } else {
        name.into()
    }
}

pub(crate) async fn publish_no_overwrite(
    source: &Path,
    destination: &Path,
    filename: &str,
) -> Result<PathBuf> {
    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let extension = path.extension().and_then(|s| s.to_str());
    for index in 0u32.. {
        let name = if index == 0 {
            filename.to_owned()
        } else if let Some(extension) = extension {
            format!("{stem} ({index}).{extension}")
        } else {
            format!("{stem} ({index})")
        };
        let target = destination.join(name);
        match fs::hard_link(source, &target).await {
            Ok(()) => return Ok(target),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                let mut input = fs::File::open(source).await?;
                match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .await
                {
                    Ok(mut output) => {
                        let copied = async {
                            tokio::io::copy(&mut input, &mut output).await?;
                            output.flush().await?;
                            output.sync_all().await
                        }
                        .await;
                        drop(output);
                        match copied {
                            Ok(()) => return Ok(target),
                            Err(error) => {
                                let _ = fs::remove_file(&target).await;
                                return Err(error.into());
                            }
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(err) => return Err(err.into()),
                }
            }
        }
    }
    Err(std::io::Error::other("no available download filename").into())
}

async fn clear_parts(work_dir: &Path) -> Result<()> {
    for index in 0..MAX_CONNECTIONS {
        remove_if_present(work_dir.join(format!("segment-{index}.part"))).await?;
    }
    remove_if_present(work_dir.join("resume.bin")).await?;
    remove_if_present(work_dir.join("assembled.part")).await?;
    Ok(())
}

async fn remove_if_present(path: PathBuf) -> Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    #[tokio::test]
    async fn missing_source_never_leaves_an_empty_published_target() {
        let directory = tempfile::tempdir().unwrap();
        let result = publish_no_overwrite(
            &directory.path().join("missing.part"),
            directory.path(),
            "finished.bin",
        )
        .await;
        assert!(result.is_err());
        assert!(!directory.path().join("finished.bin").exists());
    }
}
