use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use reqwest::{
    Client, Response, StatusCode,
    header::{self, HeaderMap, HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::Mutex,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::{Error, NewTask, Result, Store, TaskId, secrets};

const MAX_CONNECTIONS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpOptions {
    pub max_connections: usize,
    pub max_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub proxy: Option<ProxyOptions>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyOptions {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl fmt::Debug for ProxyOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyOptions")
            .field("url", &"<redacted>")
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .finish()
    }
}

impl ProxyOptions {
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.url).map_err(|_| Error::InvalidProxy)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || (self.username.is_none() && self.password.is_some())
        {
            return Err(Error::InvalidProxy);
        }
        Ok(())
    }
}

#[derive(Default)]
struct ProgressState {
    downloaded: AtomicU64,
    transferred: AtomicU64,
    merged: AtomicU64,
    saved: AtomicU64,
    save_total: AtomicU64,
    finalized_ms: AtomicU64,
    finalization_total_ms: AtomicU64,
    phase: AtomicU8,
    total: AtomicU64,
    total_known: AtomicBool,
    segments: RwLock<Vec<Arc<SegmentState>>>,
    peers: RwLock<Option<ndm_plugin_sdk::PeerStats>>,
}

struct SegmentState {
    downloaded: AtomicU64,
    total: Option<u64>,
    active: AtomicBool,
}

struct ActiveSegment(Arc<SegmentState>);

impl Drop for ActiveSegment {
    fn drop(&mut self) {
        self.0.active.store(false, Ordering::Relaxed);
    }
}

impl SegmentState {
    fn activate(segment: &Arc<Self>) -> ActiveSegment {
        segment.active.store(true, Ordering::Relaxed);
        ActiveSegment(segment.clone())
    }
}

#[derive(Clone, Default)]
pub struct DownloadProgress(Arc<ProgressState>);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentSnapshot {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProgressSnapshot {
    pub downloaded: u64,
    /// Bytes actually received during this attempt, excluding restored segments.
    pub transferred: u64,
    /// Active task time across attempts, including local processing but excluding pauses and queue time.
    pub elapsed_ms: u64,
    /// Bytes actually received across attempts, including rereads and excluding restored data.
    pub transferred_total: u64,
    pub total: Option<u64>,
    pub merged: u64,
    pub saved: u64,
    pub save_total: Option<u64>,
    /// Timestamp actually reported by the local media tool, in milliseconds.
    pub finalized_ms: u64,
    /// Known local media duration; absent when finalization cannot be measured.
    pub finalization_total_ms: Option<u64>,
    pub phase: ProgressPhase,
    pub segments: Vec<SegmentSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<ndm_plugin_sdk::PeerStats>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressPhase {
    #[default]
    Downloading,
    Merging,
    Finalizing,
    Saving,
}

impl ProgressSnapshot {
    pub fn average_bytes_per_second(&self) -> Option<u64> {
        (self.elapsed_ms > 0).then(|| {
            ((self.transferred_total as u128 * 1_000) / self.elapsed_ms as u128)
                .min(u64::MAX as u128) as u64
        })
    }
}

impl DownloadProgress {
    pub(crate) fn engine_peers(&self, stats: ndm_plugin_sdk::PeerStats) {
        *self.0.peers.write().unwrap() = Some(stats);
    }

    pub(crate) fn engine_progress(&self, progress: ndm_plugin_sdk::TransferProgress) {
        self.set_total(progress.total);
        self.0
            .downloaded
            .store(progress.downloaded, Ordering::Relaxed);
        self.0
            .transferred
            .fetch_max(progress.transferred, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            downloaded: self.0.downloaded.load(Ordering::Relaxed),
            transferred: self.0.transferred.load(Ordering::Relaxed),
            elapsed_ms: 0,
            transferred_total: self.0.transferred.load(Ordering::Relaxed),
            merged: self.0.merged.load(Ordering::Relaxed),
            saved: self.0.saved.load(Ordering::Relaxed),
            save_total: match self.0.save_total.load(Ordering::Relaxed) {
                0 => None,
                value => Some(value),
            },
            finalized_ms: self.0.finalized_ms.load(Ordering::Relaxed),
            finalization_total_ms: match self.0.finalization_total_ms.load(Ordering::Relaxed) {
                0 => None,
                value => Some(value),
            },
            phase: match self.0.phase.load(Ordering::Acquire) {
                1 => ProgressPhase::Merging,
                2 => ProgressPhase::Finalizing,
                3 => ProgressPhase::Saving,
                _ => ProgressPhase::Downloading,
            },
            total: self
                .0
                .total_known
                .load(Ordering::Acquire)
                .then(|| self.0.total.load(Ordering::Relaxed)),
            segments: self
                .0
                .segments
                .read()
                .unwrap()
                .iter()
                .map(|segment| SegmentSnapshot {
                    downloaded: segment.downloaded.load(Ordering::Relaxed),
                    total: segment.total,
                    active: segment.active.load(Ordering::Relaxed),
                })
                .collect(),
            peers: *self.0.peers.read().unwrap(),
        }
    }

    fn replace_segments(
        &self,
        totals: impl IntoIterator<Item = Option<u64>>,
    ) -> Vec<Arc<SegmentState>> {
        let segments: Vec<_> = totals
            .into_iter()
            .map(|total| {
                Arc::new(SegmentState {
                    downloaded: AtomicU64::new(0),
                    total,
                    active: AtomicBool::new(false),
                })
            })
            .collect();
        *self.0.segments.write().unwrap() = segments.clone();
        segments
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

    pub(crate) fn received(&self, bytes: u64) {
        self.0.transferred.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn add_existing(&self, bytes: u64) {
        self.0.downloaded.fetch_add(bytes, Ordering::Relaxed);
    }

    fn begin_merge(&self) {
        self.0.merged.store(0, Ordering::Relaxed);
        self.0.phase.store(1, Ordering::Release);
    }

    pub(crate) fn finalize(&self) {
        self.0.phase.store(2, Ordering::Release);
    }

    pub(crate) fn begin_finalization(&self, duration_ms: Option<u64>) {
        self.0.finalized_ms.store(0, Ordering::Relaxed);
        self.0
            .finalization_total_ms
            .store(duration_ms.unwrap_or(0), Ordering::Relaxed);
        self.finalize();
    }

    pub(crate) fn set_finalized_ms(&self, timestamp_ms: u64) {
        self.0
            .finalized_ms
            .fetch_max(timestamp_ms, Ordering::Relaxed);
    }

    pub(crate) fn resume_downloading(&self) {
        self.0.phase.store(0, Ordering::Release);
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
    match download_http_with_progress(
        &task.request,
        id,
        options,
        cancel,
        progress.clone(),
        Some(&task.work_directory()),
        None,
    )
    .await
    {
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
            max_connections: 16,
            max_bytes_per_second: None,
            proxy: None,
        }
    }
}

pub(crate) fn network_client(proxy: Option<&ProxyOptions>) -> Result<Client> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false);
    if let Some(config) = proxy {
        config.validate()?;
        let mut proxy = reqwest::Proxy::all(&config.url).map_err(|_| Error::InvalidProxy)?;
        if let Some(username) = &config.username {
            proxy = proxy.basic_auth(username, config.password.as_deref().unwrap_or(""));
        }
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(network)
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
    download_http_with_progress(
        request,
        id,
        options,
        cancel,
        DownloadProgress::default(),
        None,
        None,
    )
    .await
}

/// Download with the same range, retry, merge and progress engine as normal tasks.
/// The client supplies proxy settings and can follow redirects with a custom host
/// policy. With redirects disabled on the client, normal explicit core redirects apply.
pub async fn download_http_with_client(
    request: &NewTask,
    id: TaskId,
    options: &HttpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
    client: Client,
) -> Result<PathBuf> {
    download_http_with_progress(request, id, options, cancel, progress, None, Some(client)).await
}

async fn download_http_with_progress(
    request: &NewTask,
    id: TaskId,
    options: &HttpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
    work_directory: Option<&std::path::Path>,
    client: Option<Client>,
) -> Result<PathBuf> {
    progress.0.downloaded.store(0, Ordering::Relaxed);
    progress.0.transferred.store(0, Ordering::Relaxed);
    progress.0.phase.store(0, Ordering::Release);
    progress.set_total(None);
    let client = match client {
        Some(client) => client,
        None => network_client(options.proxy.as_ref())?,
    };
    let headers = request_headers(request)?;
    fs::create_dir_all(&request.destination).await?;
    let work_dir = work_directory
        .map(std::path::Path::to_owned)
        .unwrap_or_else(|| request.destination.join(format!(".ndm-next-{id}")));
    fs::create_dir_all(&work_dir).await?;
    crate::cache::validate_directory(&work_dir)?;
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
    let discovered_filename = disposition_filename(probe.headers()).or_else(|| {
        probe
            .url()
            .path_segments()
            .and_then(|mut parts| parts.next_back())
            .filter(|segment| !segment.is_empty())
            .and_then(|segment| {
                percent_encoding::percent_decode_str(segment)
                    .decode_utf8()
                    .ok()
            })
            .map(|name| name.into_owned())
    });
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
        if request
            .expected_size
            .is_some_and(|expected| expected != total)
        {
            return Err(Error::InvalidHttpResponse);
        }
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
        let segments = progress.replace_segments((0..connections).map(|index| {
            let start = ((total as u128 * index as u128) / connections as u128) as u64;
            let end = ((total as u128 * (index + 1) as u128) / connections as u128) as u64;
            Some(end - start)
        }));
        for (index, segment) in segments.iter().enumerate() {
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
                segment.clone(),
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
                download_full(
                    response,
                    &assembled,
                    &limiter,
                    &cancel,
                    &progress,
                    request.expected_size,
                )
                .await?;
            }
            Some(error) => return Err(error),
            None => {
                assemble_segments(
                    &work_dir,
                    &assembled,
                    connections,
                    total,
                    &cancel,
                    &progress,
                )
                .await?;
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
        download_full(
            response,
            &assembled,
            &limiter,
            &cancel,
            &progress,
            request.expected_size,
        )
        .await?;
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if let Some(size) = request.expected_size
        && fs::metadata(&assembled).await?.len() != size
    {
        return Err(Error::InvalidHttpResponse);
    }

    let filename = safe_filename(
        request
            .suggested_filename
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .or(discovered_filename.as_deref())
            .unwrap_or("download"),
    );
    let target = publish_download(
        &assembled,
        &request.destination,
        &filename,
        &progress,
        &cancel,
    )
    .await?;
    // The target is already published; cleanup failures must not turn a
    // successful transfer into a failed task and cause a duplicate retry.
    let _ = clear_parts(&work_dir).await;
    let _ = fs::remove_dir(&work_dir).await;
    Ok(target)
}

async fn assemble_segments(
    work_dir: &Path,
    assembled: &Path,
    connections: usize,
    total: u64,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
) -> Result<()> {
    progress.begin_merge();
    let mut merged = fs::File::create(assembled).await?;
    let mut buffer = vec![0_u8; 256 * 1024];
    for index in 0..connections {
        let mut part = fs::File::open(work_dir.join(format!("segment-{index}.part"))).await?;
        copy_segment(&mut part, &mut merged, &mut buffer, cancel, progress).await?;
    }
    progress.finalize();
    merged.flush().await?;
    if merged.metadata().await?.len() != total {
        return Err(Error::InvalidHttpResponse);
    }
    merged.sync_all().await?;
    Ok(())
}

async fn copy_segment(
    input: &mut (impl AsyncRead + Unpin),
    output: &mut (impl AsyncWrite + Unpin),
    buffer: &mut [u8],
    cancel: &CancellationToken,
    progress: &DownloadProgress,
) -> Result<()> {
    loop {
        let count = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            count = input.read(buffer) => count?,
        };
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count]).await?;
        progress.0.merged.fetch_add(count as u64, Ordering::Relaxed);
    }
    Ok(())
}

async fn download_full(
    response: Response,
    assembled: &Path,
    limiter: &RateLimiter,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
    expected_size: Option<u64>,
) -> Result<()> {
    if response.status() != StatusCode::OK {
        return Err(Error::HttpStatus(response.status().as_u16()));
    }
    let expected = response.content_length();
    let maximum = match (expected, expected_size) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    };
    progress.set_total(expected);
    let segment = progress
        .replace_segments([expected.or(expected_size)])
        .remove(0);
    let _active = SegmentState::activate(&segment);
    let mut file = fs::File::create(assembled).await?;
    let result = write_response(
        response,
        &mut file,
        limiter,
        cancel,
        progress,
        Some(&segment),
        maximum,
    )
    .await;
    if result.is_ok() {
        progress.finalize();
    }
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
    segment: Arc<SegmentState>,
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
    progress.add_existing(present);
    segment.downloaded.store(present, Ordering::Relaxed);
    if present == length {
        file.sync_data().await?;
        return Ok(());
    }
    let _active = SegmentState::activate(&segment);
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
        let result = write_response(
            response,
            &mut file,
            &limiter,
            &cancel,
            &progress,
            Some(&segment),
            Some(length - present),
        )
        .await;
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
    segment: Option<&SegmentState>,
    maximum: Option<u64>,
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
        progress.received(chunk.len() as u64);
        let next = total
            .checked_add(chunk.len() as u64)
            .ok_or(Error::InvalidHttpResponse)?;
        if maximum.is_some_and(|maximum| next > maximum) {
            return Err(Error::InvalidHttpResponse);
        }
        limiter.wait(chunk.len(), cancel).await?;
        file.write_all(&chunk).await?;
        progress.add(chunk.len() as u64);
        if let Some(segment) = segment {
            segment
                .downloaded
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        total = next;
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

fn disposition_filename(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::CONTENT_DISPOSITION)?.to_str().ok()?;
    if value.len() > 8192 {
        return None;
    }
    // Split only at unquoted semicolons, retaining escaped characters in values.
    let mut fields = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
        } else if ch == ';' && !quoted {
            fields.push(&value[start..index]);
            start = index + 1;
        }
    }
    if quoted {
        return None;
    }
    fields.push(&value[start..]);
    let (mut plain, mut extended) = (None, None);
    for field in fields.into_iter().skip(1) {
        let Some((key, raw)) = field.trim().split_once('=') else {
            continue;
        };
        let raw = raw.trim();
        let unquoted = raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(raw);
        if key.trim().eq_ignore_ascii_case("filename*") {
            let mut parts = unquoted.splitn(3, '\'');
            if parts
                .next()
                .is_some_and(|charset| charset.eq_ignore_ascii_case("utf-8"))
                && parts.next().is_some()
                && let Some(encoded) = parts.next()
            {
                extended = percent_encoding::percent_decode_str(encoded)
                    .decode_utf8()
                    .ok()
                    .map(|value| value.into_owned());
            }
        } else if key.trim().eq_ignore_ascii_case("filename") {
            let mut name = String::new();
            let mut chars = unquoted.chars();
            while let Some(ch) = chars.next() {
                if ch == '\\' {
                    name.push(chars.next().unwrap_or(ch));
                } else {
                    name.push(ch);
                }
            }
            plain = Some(name);
        }
    }
    extended
        .or(plain)
        .filter(|name| !name.trim().is_empty() && !name.chars().any(char::is_control))
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

/// A cross-volume cache must stage the complete file on the destination volume
/// before publishing its final name. Existing files are never overwritten.
pub(crate) async fn publish_download(
    source: &Path,
    destination: &Path,
    filename: &str,
    progress: &DownloadProgress,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    match publish_local(source, destination, filename).await {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            publish_staged(source, destination, filename, progress, cancel).await
        }
        Err(error) => Err(error.into()),
    }
}

async fn publish_staged(
    source: &Path,
    destination: &Path,
    filename: &str,
    progress: &DownloadProgress,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    let (from, to, progress, cancel) = (
        source.to_owned(),
        destination.to_owned(),
        progress.clone(),
        cancel.clone(),
    );
    let staged = tokio::task::spawn_blocking(move || -> Result<tempfile::NamedTempFile> {
        use std::io::{Read, Write};
        let mut input = std::fs::File::open(&from)?;
        let total = input.metadata()?.len();
        progress.0.saved.store(0, Ordering::Relaxed);
        progress.0.save_total.store(total, Ordering::Relaxed);
        progress.0.phase.store(3, Ordering::Release);
        let mut staged = tempfile::Builder::new()
            .prefix(".ndm-next-publish-")
            .suffix(".part")
            .tempfile_in(to)?;
        let mut buffer = vec![0; 512 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let bytes = input.read(&mut buffer)?;
            if bytes == 0 {
                break;
            }
            staged.write_all(&buffer[..bytes])?;
            progress.0.saved.fetch_add(bytes as u64, Ordering::Relaxed);
        }
        staged.as_file().sync_all()?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(staged)
    })
    .await
    .map_err(std::io::Error::other)??;
    let target = publish_local(staged.path(), destination, filename).await?;
    let _ = fs::remove_file(source).await;
    Ok(target)
}

#[cfg(test)]
async fn publish_no_overwrite(
    source: &Path,
    destination: &Path,
    filename: &str,
) -> Result<PathBuf> {
    publish_download(
        source,
        destination,
        filename,
        &DownloadProgress::default(),
        &CancellationToken::new(),
    )
    .await
}

async fn publish_local(
    source: &Path,
    destination: &Path,
    filename: &str,
) -> std::io::Result<PathBuf> {
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
        match publish_file(source, &target).await {
            Ok(()) => return Ok(target),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::other("no available download filename"))
}

#[cfg(windows)]
async fn publish_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let wide_path = |path: &Path| -> std::io::Result<Vec<u16>> {
        let mut path: Vec<u16> = path.as_os_str().encode_wide().collect();
        if path.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        path.push(0);
        Ok(path)
    };
    let source = wide_path(source)?;
    let target = wide_path(target)?;
    tokio::task::spawn_blocking(move || {
        // With no flags, MoveFileExW neither replaces an existing target nor
        // copies across volumes.
        // All callers prepare the complete file inside the destination folder.
        // SAFETY: both paths are valid, owned, NUL-terminated UTF-16 buffers.
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    })
    .await
    .map_err(std::io::Error::other)?
}

#[cfg(not(windows))]
async fn publish_file(source: &Path, target: &Path) -> std::io::Result<()> {
    // Non-Windows builds are used for development only. A hard link publishes
    // the already complete file without ever replacing an existing target.
    fs::hard_link(source, target).await
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

    #[test]
    fn restored_bytes_are_not_counted_as_network_traffic() {
        let progress = DownloadProgress::default();
        progress.add_existing(1_000);
        progress.received(250);
        progress.add(250);
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.downloaded, 1_250);
        assert_eq!(snapshot.transferred, 250);
    }

    #[tokio::test]
    async fn assembly_reports_actual_bytes_and_preserves_segments() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("segment-0.part"), b"first ").unwrap();
        std::fs::write(directory.path().join("segment-1.part"), b"second").unwrap();
        let assembled = directory.path().join("assembled.part");
        let progress = DownloadProgress::default();
        assemble_segments(
            directory.path(),
            &assembled,
            2,
            12,
            &CancellationToken::new(),
            &progress,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(assembled).unwrap(), b"first second");
        assert!(directory.path().join("segment-0.part").exists());
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.merged, 12);
        assert_eq!(snapshot.phase, ProgressPhase::Finalizing);
    }

    #[tokio::test]
    async fn cache_export_stages_complete_files_and_never_overwrites() {
        let cache = tempfile::tempdir().unwrap();
        let other_volume = std::env::var_os("NDM_TEST_OTHER_VOLUME");
        let destination = if let Some(root) = &other_volume {
            tempfile::tempdir_in(root).unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let source = cache.path().join("assembled.part");
        let content = vec![0x7b; 1_200_000];
        std::fs::write(&source, &content).unwrap();
        std::fs::write(destination.path().join("file.bin"), b"keep").unwrap();
        let progress = DownloadProgress::default();
        let token = CancellationToken::new();
        let result = if other_volume.is_some() {
            publish_download(&source, destination.path(), "file.bin", &progress, &token)
                .await
                .unwrap()
        } else {
            publish_staged(&source, destination.path(), "file.bin", &progress, &token)
                .await
                .unwrap()
        };
        assert_eq!(result.file_name().unwrap(), "file (1).bin");
        assert_eq!(std::fs::read(&result).unwrap(), content);
        assert_eq!(
            std::fs::read(destination.path().join("file.bin")).unwrap(),
            b"keep"
        );
        assert!(!source.exists());
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.phase, ProgressPhase::Saving);
        assert_eq!(snapshot.saved, 1_200_000);
        assert_eq!(snapshot.save_total, Some(1_200_000));
        assert_eq!(std::fs::read_dir(destination.path()).unwrap().count(), 2);
    }

    #[tokio::test]
    async fn cancelled_cache_export_keeps_source_and_removes_staging_files() {
        let cache = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let source = cache.path().join("assembled.part");
        std::fs::write(&source, b"resumable").unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = publish_staged(
            &source,
            destination.path(),
            "file.bin",
            &DownloadProgress::default(),
            &cancel,
        )
        .await;
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(std::fs::read(&source).unwrap(), b"resumable");
        assert_eq!(std::fs::read_dir(destination.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn assembly_updates_progress_before_the_input_finishes() {
        let (mut producer, mut reader) = tokio::io::duplex(64);
        let progress = DownloadProgress::default();
        progress.begin_merge();
        let observed = progress.clone();
        let copy = tokio::spawn(async move {
            let mut output = tokio::io::sink();
            let mut buffer = [0; 16];
            copy_segment(
                &mut reader,
                &mut output,
                &mut buffer,
                &CancellationToken::new(),
                &observed,
            )
            .await
        });
        producer.write_all(b"first").await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while progress.snapshot().merged != 5 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("merge should report bytes before the stream closes");
        assert_eq!(progress.snapshot().phase, ProgressPhase::Merging);
        producer.write_all(b"second").await.unwrap();
        producer.shutdown().await.unwrap();
        copy.await.unwrap().unwrap();
        assert_eq!(progress.snapshot().merged, 11);
    }

    #[tokio::test]
    async fn cancelled_assembly_keeps_reusable_segments() {
        let directory = tempfile::tempdir().unwrap();
        let part = directory.path().join("segment-0.part");
        std::fs::write(&part, b"keep this part").unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = assemble_segments(
            directory.path(),
            &directory.path().join("assembled.part"),
            1,
            14,
            &cancel,
            &DownloadProgress::default(),
        )
        .await;
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(std::fs::read(part).unwrap(), b"keep this part");
    }

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

    #[cfg(windows)]
    #[tokio::test]
    async fn publication_moves_a_complete_file_without_overwriting_existing_data() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("finished.part");
        let existing = directory.path().join("finished.bin");
        std::fs::write(&source, b"complete transfer").unwrap();
        std::fs::write(&existing, b"existing file").unwrap();

        let published = publish_no_overwrite(&source, directory.path(), "finished.bin")
            .await
            .unwrap();
        assert_eq!(std::fs::read(existing).unwrap(), b"existing file");
        assert_eq!(std::fs::read(&published).unwrap(), b"complete transfer");
        assert_eq!(published.file_name().unwrap(), "finished (1).bin");
        assert!(
            !source.exists(),
            "publication should move the complete file"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cross_volume_publication_never_creates_a_partial_target() {
        let Some(other_volume) = std::env::var_os("NDM_TEST_OTHER_VOLUME") else {
            return;
        };
        let source_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir_in(other_volume).unwrap();
        let source = source_dir.path().join("finished.part");
        std::fs::write(&source, b"complete transfer").unwrap();
        let result = publish_no_overwrite(&source, target_dir.path(), "finished.bin").await;
        assert!(
            result.is_err(),
            "cross-volume publication must not copy directly to the target"
        );
        assert!(!target_dir.path().join("finished.bin").exists());
        assert_eq!(std::fs::read(source).unwrap(), b"complete transfer");
    }
}
