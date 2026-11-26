use std::{
    collections::VecDeque,
    fmt,
    io::SeekFrom,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::diagnostics::{SegmentFailure, SegmentIssue};
use futures_util::{StreamExt, stream::FuturesUnordered};
use reqwest::{
    Client, Response, StatusCode, Url,
    header::{self, HeaderMap, HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt},
    sync::Mutex,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::{
    DownloadFailure, Error, HttpDiagnostics, HttpRangeMode, HttpStage, NewTask, ProxyMode, Result,
    Store, TaskActivity, TaskId, diagnostics::Routing, secrets, validate_user_agent,
};

mod repository;
use repository::Sha256Bytes;
pub use repository::http_filename_hint;

const MAX_CONNECTIONS: usize = 16;
const RANGE_WAVES: usize = 4;
const MAX_PARTS: usize = MAX_CONNECTIONS * RANGE_WAVES;
const MIN_PARALLEL_BYTES: u64 = 32 * 1024;
const MIN_QUEUED_PART_BYTES: u64 = 2 * 1024 * 1024;

pub const CHROME_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpOptions {
    pub max_connections: usize,
    pub max_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub proxy: Option<ProxyOptions>,
    #[serde(default)]
    pub proxy_mode: ProxyMode,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tun_egress: Option<crate::TunEgress>,
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
    estimated_total: AtomicU64,
    segments: RwLock<Vec<Arc<SegmentState>>>,
    peers: RwLock<Option<ndm_plugin_sdk::PeerStats>>,
    http: RwLock<Option<HttpDiagnostics>>,
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
    /// Advisory total used for display. Keep size and integrity checks on exact values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_total: Option<u64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http: Option<HttpDiagnostics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<TaskActivity>,
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

    pub(crate) fn engine_progress(
        &self,
        progress: ndm_plugin_sdk::TransferProgress,
        estimated_total: Option<u64>,
    ) {
        let estimate = estimated_total
            .filter(|&bytes| progress.total.is_none() && bytes >= progress.downloaded)
            .unwrap_or_default();
        self.0.estimated_total.store(estimate, Ordering::Relaxed);
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
            estimated_total: match self.0.estimated_total.load(Ordering::Relaxed) {
                0 => None,
                bytes => Some(bytes),
            },
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
            http: self.0.http.read().unwrap().clone(),
            activity: None,
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
    execute_http_with_title(store, id, options, cancel, progress, || {}).await
}

pub(crate) async fn execute_http_with_title(
    store: &Store,
    id: TaskId,
    options: &HttpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
    title_changed: impl Fn() + Sync,
) -> Result<PathBuf> {
    let task = store.get(id)?.ok_or(Error::NotFound)?;
    store.start(id)?;
    let report = |name: &str| {
        if store.set_engine_title(id, name).unwrap_or(false) {
            title_changed();
        }
    };
    match download_http_with_progress(
        &task.request,
        id,
        options,
        cancel,
        progress.clone(),
        HttpContext {
            work_directory: Some(&task.work_directory()),
            report_filename: Some(&report),
            client: None,
        },
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
            let failure = DownloadFailure::from_error(&error);
            {
                let mut http = progress.0.http.write().unwrap();
                let diagnostics = http.get_or_insert_default();
                diagnostics.failure = Some(failure);
                if let Error::HttpStatus(status) = &error {
                    diagnostics.http_status = Some(*status);
                }
            }
            // Store terminal error and trace together, including for direct core callers.
            store.fail_http(id, failure.message(), &progress.snapshot())?;
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
            proxy_mode: ProxyMode::Inherit,
            tun_egress: None,
            user_agent: None,
        }
    }
}

pub(crate) fn network_client(
    proxy: Option<&ProxyOptions>,
    mode: ProxyMode,
    user_agent: Option<&str>,
    egress: Option<&crate::TunEgress>,
) -> Result<Client> {
    let user_agent = user_agent.unwrap_or(CHROME_USER_AGENT);
    validate_user_agent(user_agent)?;
    let mut builder = Client::builder()
        .user_agent(user_agent)
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false);
    if mode == ProxyMode::TunDirect {
        builder = builder.no_proxy();
        match egress.ok_or(Error::TunEgressUnavailable)? {
            crate::TunEgress::LocalPort(port) => {
                let listener = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
                    .map_err(|_| Error::TunEgressUnavailable)?;
                builder = builder.proxy(listener);
            }
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            crate::TunEgress::Interface(name) if crate::TunEgress::parse(name).is_ok() => {
                builder = builder.interface(name);
            }
            _ => return Err(Error::TunEgressUnavailable),
        }
    } else if matches!(mode, ProxyMode::Direct | ProxyMode::SystemRoute) {
        builder = builder.no_proxy();
    } else if let Some(config) = proxy {
        config.validate()?;
        let mut proxy = reqwest::Proxy::all(&config.url).map_err(|_| Error::InvalidProxy)?;
        if let Some(username) = &config.username {
            proxy = proxy.basic_auth(username, config.password.as_deref().unwrap_or(""));
        }
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(network)
}

#[derive(Clone)]
struct HttpTrace<'a> {
    progress: &'a DownloadProgress,
    routing: &'a Routing,
    details: HttpDiagnostics,
    linked_sha256: Option<Sha256Bytes>,
    retries: u32,
}

impl HttpTrace<'_> {
    fn record(&self) {
        *self.progress.0.http.write().unwrap() = Some(self.details.clone());
    }

    fn stage(&mut self, stage: HttpStage) {
        self.details.stage = stage;
        self.record();
    }

    async fn send(
        &mut self,
        stage: HttpStage,
        client: &Client,
        builder: reqwest::RequestBuilder,
        cancel: &CancellationToken,
    ) -> Result<Response> {
        self.details.http_status = None;
        self.details.redirect_count = 0;
        self.details.effective_proxy = None;
        self.stage(stage);
        let mut retries = 0;
        let result = send_observed(
            client,
            builder,
            cancel,
            &mut retries,
            false,
            |url, response, redirects| {
                let status = response.map(Response::status);
                if stage == HttpStage::RangeProbe
                    && let Some(response) = response.filter(|r| r.status().is_redirection())
                    && let Some(digest) = repository::hf_lfs_digest(url, response.headers())
                {
                    self.linked_sha256 = Some(digest);
                }
                self.details.stage = if status.is_some_and(|status| status.is_redirection()) {
                    HttpStage::Redirect
                } else {
                    stage
                };
                self.details.http_status = status.map(|status| status.as_u16());
                self.details.redirect_count = redirects;
                self.details.effective_proxy = self.routing.for_url(url);
                self.record();
            },
        )
        .await;
        self.retries += retries;
        result
    }
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
struct ResumeManifest {
    validator: RangeValidator,
    resource_url: String,
    total: u64,
    // Older manifests used the connection count as the number of equal parts.
    #[serde(rename = "connections")]
    parts: usize,
}

// The string variant preserves manifests written by 0.6.3 and earlier.
#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum RangeValidator {
    Etag(String),
    Digest { sha256: Sha256Bytes },
}

impl ResumeManifest {
    fn bounds(&self, index: usize) -> std::ops::Range<u64> {
        let offset = |i: usize| (self.total as u128 * i as u128 / self.parts as u128) as u64;
        offset(index)..offset(index + 1)
    }
}

fn part_count(total: u64, connections: usize, request_time: Duration) -> usize {
    let slots = (total / MIN_PARALLEL_BYTES).clamp(1, connections as u64);
    if slots == 1 {
        return 1;
    }
    // Budget 64 KiB of work per millisecond spent establishing the probe,
    // so slow redirects/RTTs do not turn a medium file into many small GETs.
    let part_floor = (request_time.as_millis() * 64 * 1024)
        .clamp(MIN_QUEUED_PART_BYTES as u128, 64 * 1024 * 1024) as u64;
    let waves = (total / (part_floor * slots)).clamp(1, RANGE_WAVES as u64);
    (slots * waves) as usize
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
        HttpContext::default(),
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
    download_http_with_progress(
        request,
        id,
        options,
        cancel,
        progress,
        HttpContext {
            client: Some(client),
            ..Default::default()
        },
    )
    .await
}

#[derive(Default)]
struct HttpContext<'a> {
    work_directory: Option<&'a Path>,
    client: Option<Client>,
    report_filename: Option<&'a (dyn Fn(&str) + Sync)>,
}

async fn download_http_with_progress(
    request: &NewTask,
    id: TaskId,
    options: &HttpOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
    context: HttpContext<'_>,
) -> Result<PathBuf> {
    progress.0.downloaded.store(0, Ordering::Relaxed);
    progress.0.transferred.store(0, Ordering::Relaxed);
    progress.0.phase.store(0, Ordering::Release);
    progress.set_total(None);
    let routing = if context.client.is_none() {
        Routing::new(
            options.proxy_mode,
            options.proxy.as_ref(),
            options.tun_egress.as_ref(),
        )
    } else {
        Routing::CustomClient
    };
    let mut trace = HttpTrace {
        progress: &progress,
        routing: &routing,
        details: HttpDiagnostics::default(),
        linked_sha256: None,
        retries: 0,
    };
    trace.record();
    let client = match context.client {
        Some(_)
            if matches!(
                options.proxy_mode,
                ProxyMode::TunDirect | ProxyMode::SystemRoute
            ) =>
        {
            return Err(Error::TunEgressUnavailable);
        }
        Some(client) => client,
        None => network_client(
            options.proxy.as_ref(),
            options.proxy_mode,
            options.user_agent.as_deref(),
            options.tun_egress.as_ref(),
        )?,
    };
    let mut headers = request_headers(request)?;
    headers
        .entry(header::ACCEPT_ENCODING)
        .or_insert(HeaderValue::from_static("identity"));
    fs::create_dir_all(&request.destination).await?;
    let work_dir = context
        .work_directory
        .map(std::path::Path::to_owned)
        .unwrap_or_else(|| request.destination.join(format!(".ndm-next-{id}")));
    fs::create_dir_all(&work_dir).await?;
    crate::cache::validate_directory(&work_dir)?;
    let limiter = RateLimiter::new(options.max_bytes_per_second);

    let probe_started = Instant::now();
    let probe = trace
        .send(
            HttpStage::RangeProbe,
            &client,
            client
                .get(&request.url)
                .headers(headers.clone())
                .header(header::RANGE, "bytes=0-0"),
            &cancel,
        )
        .await?;
    let probe_time = probe_started.elapsed();
    trace.details.range_probe_status = Some(probe.status().as_u16());
    trace.record();
    let source = reqwest::Url::parse(&request.url).map_err(|_| Error::InvalidHttpResponse)?;
    let explicit_name = request
        .suggested_filename
        .as_deref()
        .filter(|name| !name.trim().is_empty());
    let mut discovered_filename = None;
    let mut attachment = None;
    let mut observe_name = |response: &Response| {
        if !response.status().is_success() {
            return;
        }
        if let Some(value) = disposition_filename(response.headers()) {
            attachment = Some(value);
        }
        let name = attachment
            .clone()
            .or_else(|| repository::source_filename(&source))
            .or_else(|| http_filename_hint(response.url()))
            .or_else(|| http_filename_hint(&source))
            .map(|name| safe_filename(&name));
        if explicit_name.is_none()
            && name != discovered_filename
            && let (Some(report), Some(name)) = (context.report_filename, &name)
        {
            report(name);
        }
        // An opaque redirect path may be signed. Only use it as the last-resort output name.
        discovered_filename = name.or_else(|| response_basename(response));
    };
    observe_name(&probe);
    let range_total = if probe.status() == StatusCode::PARTIAL_CONTENT {
        probe
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range)
            .filter(|&(start, end, total)| start == 0 && end == 0 && total > 0)
            .map(|(_, _, total)| total)
    } else {
        None
    };
    if let (Some(expected), Some(total)) = (request.expected_size, range_total)
        && expected != total
    {
        trace.stage(HttpStage::IntegrityCheck);
        return Err(Error::InvalidHttpResponse);
    }
    let expected_size = request.expected_size.or(range_total);
    let mut digest = probe
        .status()
        .is_success()
        .then(|| repository::representation_digest(probe.headers()))
        .flatten()
        .or(trace.linked_sha256);
    if digest.is_none()
        && let Some(total) = range_total
    {
        digest = repository::modelscope_digest(&source, total, &client, &headers, &cancel).await;
    }
    let validator = digest
        .map(|sha256| RangeValidator::Digest { sha256 })
        .or_else(|| strong_etag(probe.headers()).map(RangeValidator::Etag));
    let ranged = range_total.zip(validator);
    trace.details.range_mode = Some(match &ranged {
        Some(_) if options.max_connections <= 1 => HttpRangeMode::ConnectionLimit,
        Some((_, RangeValidator::Etag(_))) => HttpRangeMode::StrongEtag,
        Some((_, RangeValidator::Digest { .. })) => HttpRangeMode::VerifiedSha256,
        None if probe.status() == StatusCode::OK => HttpRangeMode::RangeIgnored,
        None if probe.status() != StatusCode::PARTIAL_CONTENT => HttpRangeMode::RangeRejected,
        None if range_total.is_none() => HttpRangeMode::InvalidContentRange,
        None if options.max_connections <= 1 => HttpRangeMode::ConnectionLimit,
        None => HttpRangeMode::MissingValidator,
    });
    trace.record();

    let assembled = work_dir.join("assembled.part");
    if let Some((total, validator)) = ranged {
        let resource_url = if digest.is_some() {
            source.as_str().to_owned()
        } else {
            range_resource_key(trace.details.redirect_count > 0, probe.url())
        };
        drop(probe);
        progress.set_total(Some(total));
        let connections = options.max_connections.clamp(1, MAX_CONNECTIONS);
        let manifest_path = work_dir.join("resume.bin");
        let existing = std::fs::read(&manifest_path)
            .ok()
            .and_then(|bytes| secrets::unprotect(&bytes).ok())
            .and_then(|bytes| serde_json::from_slice::<ResumeManifest>(&bytes).ok())
            .filter(|saved| {
                saved.validator == validator
                    && saved.resource_url == resource_url
                    && saved.total == total
                    && (1..=MAX_PARTS).contains(&saved.parts)
                    && saved.parts as u64 <= total
            });
        let resuming = existing.is_some();
        let manifest = if let Some(saved) = existing {
            saved
        } else {
            clear_parts(&work_dir).await?;
            let manifest = ResumeManifest {
                validator,
                resource_url,
                total,
                parts: part_count(total, connections, probe_time),
            };
            let protected = secrets::protect(&serde_json::to_vec(&manifest)?)?;
            let mut file = fs::File::create(&manifest_path).await?;
            file.write_all(&protected).await?;
            file.sync_all().await?;
            manifest
        };
        let segment_cancel = cancel.child_token();
        let segments = progress.replace_segments((0..manifest.parts).map(|index| {
            let range = manifest.bounds(index);
            Some(range.end - range.start)
        }));
        if resuming {
            for (index, segment) in segments.iter().enumerate() {
                let path = work_dir.join(format!("segment-{index}.part"));
                match fs::metadata(path).await {
                    Ok(info) if segment.total.is_some_and(|length| info.len() <= length) => {
                        segment.downloaded.store(info.len(), Ordering::Relaxed);
                        progress.add_existing(info.len());
                    }
                    Ok(_) => {} // An oversized part is reset when opened.
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err.into()),
                }
            }
        }
        let source = RangeSource {
            client: &client,
            url: &request.url,
            headers: &headers,
            manifest: &manifest,
            limiter: &limiter,
            cancel: &segment_cancel,
        };
        let result = source
            .download(&work_dir, &segments, connections, &mut trace)
            .await;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        match result {
            Err(error) => {
                if !range_failure_can_retry_full(&error) {
                    return Err(error);
                }
                trace.details.range_mode = Some(HttpRangeMode::SegmentFallback);
                progress.0.downloaded.store(0, Ordering::Relaxed);
                let response = trace
                    .send(
                        HttpStage::FullDownload,
                        &client,
                        client.get(&request.url).headers(headers),
                        &cancel,
                    )
                    .await?;
                observe_name(&response);
                let fallback = download_full(
                    response,
                    &assembled,
                    &limiter,
                    &cancel,
                    expected_size,
                    digest,
                    &mut trace,
                )
                .await;
                if matches!(fallback, Err(Error::ChecksumMismatch)) {
                    clear_parts(&work_dir).await?;
                }
                fallback?;
            }
            Ok(()) => {
                trace.stage(HttpStage::IntegrityCheck);
                let result =
                    assemble_segments(&work_dir, &assembled, &manifest, &cancel, &progress).await;
                if matches!(result, Err(Error::ChecksumMismatch)) {
                    clear_parts(&work_dir).await?;
                    progress.0.downloaded.store(0, Ordering::Relaxed);
                    progress.replace_segments([]);
                }
                result?;
            }
        }
    } else {
        let response = if probe.status() == StatusCode::OK {
            probe
        } else if probe.status() == StatusCode::PARTIAL_CONTENT
            || range_failure_can_retry_full(&Error::HttpStatus(probe.status().as_u16()))
        {
            drop(probe);
            trace
                .send(
                    HttpStage::FullDownload,
                    &client,
                    client.get(&request.url).headers(headers),
                    &cancel,
                )
                .await?
        } else {
            return Err(Error::HttpStatus(probe.status().as_u16()));
        };
        // A 200 probe is also the full response.
        if trace.details.range_probe_status != Some(200) {
            observe_name(&response);
        }
        download_full(
            response,
            &assembled,
            &limiter,
            &cancel,
            expected_size,
            digest,
            &mut trace,
        )
        .await?;
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    trace.stage(HttpStage::IntegrityCheck);
    if let Some(size) = expected_size
        && fs::metadata(&assembled).await?.len() != size
    {
        return Err(Error::InvalidHttpResponse);
    }

    let filename = safe_filename(
        explicit_name
            .or(discovered_filename.as_deref())
            .unwrap_or("download"),
    );
    trace.stage(HttpStage::Publish);
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

// Some endpoints reject Range while accepting an ordinary GET. Keep the
// same URL, credentials and proxy policy for the single streaming fallback.
fn range_failure_can_retry_full(error: &Error) -> bool {
    matches!(
        error,
        Error::InvalidHttpResponse | Error::HttpStatus(400 | 403 | 405 | 416 | 501)
    )
}

async fn assemble_segments(
    work_dir: &Path,
    assembled: &Path,
    manifest: &ResumeManifest,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
) -> Result<()> {
    progress.begin_merge();
    let mut merged = fs::File::create(assembled).await?;
    let mut buffer = vec![0_u8; 256 * 1024];
    let mut hasher = matches!(manifest.validator, RangeValidator::Digest { .. }).then(Sha256::new);
    for index in 0..manifest.parts {
        let mut part = fs::File::open(work_dir.join(format!("segment-{index}.part"))).await?;
        copy_segment(
            &mut part,
            &mut merged,
            &mut buffer,
            cancel,
            progress,
            hasher.as_mut(),
        )
        .await?;
    }
    progress.finalize();
    merged.flush().await?;
    if merged.metadata().await?.len() != manifest.total {
        return Err(Error::InvalidHttpResponse);
    }
    if let (Some(hasher), RangeValidator::Digest { sha256 }) = (hasher, &manifest.validator)
        && <Sha256Bytes>::from(hasher.finalize()) != *sha256
    {
        return Err(Error::ChecksumMismatch);
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
    mut hasher: Option<&mut Sha256>,
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
        if let Some(h) = hasher.as_mut() {
            h.update(&buffer[..count]);
        }
        progress.0.merged.fetch_add(count as u64, Ordering::Relaxed);
    }
    Ok(())
}

async fn download_full(
    response: Response,
    assembled: &Path,
    limiter: &RateLimiter,
    cancel: &CancellationToken,
    expected_size: Option<u64>,
    sha256: Option<Sha256Bytes>,
    trace: &mut HttpTrace<'_>,
) -> Result<()> {
    if response.status() != StatusCode::OK {
        return Err(Error::HttpStatus(response.status().as_u16()));
    }
    let sha256 = sha256.or_else(|| repository::representation_digest(response.headers()));
    trace.stage(HttpStage::FullDownload);
    let progress = trace.progress;
    let expected = response.content_length();
    if let (Some(response_size), Some(expected_size)) = (expected, expected_size)
        && response_size != expected_size
    {
        trace.stage(HttpStage::IntegrityCheck);
        return Err(Error::InvalidHttpResponse);
    }
    let maximum = match (expected, expected_size) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    };
    progress.set_total(expected.or(expected_size));
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
        BodyIntegrity { maximum, sha256 },
    )
    .await;
    if result.is_ok() {
        progress.finalize();
    }
    file.flush().await?;
    if matches!(
        &result,
        Err(Error::InvalidHttpResponse | Error::ChecksumMismatch)
    ) {
        trace.stage(HttpStage::IntegrityCheck);
    }
    if matches!(result, Err(Error::ChecksumMismatch)) {
        drop(file);
        remove_if_present(assembled.to_owned()).await?;
        progress.0.downloaded.store(0, Ordering::Relaxed);
        progress.replace_segments([]);
        return Err(Error::ChecksumMismatch);
    }
    let written = result?;
    if expected
        .or(expected_size)
        .is_some_and(|expected| expected != written)
    {
        trace.stage(HttpStage::IntegrityCheck);
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

/// Keep device and language hints for media CDNs, while scoping credentials to
/// the source origin. Referer across origins contains only the page origin.
pub(crate) fn media_headers(headers: &HeaderMap, source: &Url, target: &Url) -> HeaderMap {
    if target.origin() == source.origin() {
        return headers.clone();
    }
    let mut safe = HeaderMap::new();
    for name in [header::USER_AGENT, header::ACCEPT_LANGUAGE] {
        if let Some(value) = headers.get(&name) {
            safe.insert(name, value.clone());
        }
    }
    if let Some(page) = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Url::parse(value).ok())
        .filter(|page| {
            page.scheme() == "http" || page.scheme() == "https" && target.scheme() == "https"
        })
        && let Ok(origin) =
            HeaderValue::from_str(&format!("{}/", page.origin().ascii_serialization()))
    {
        safe.insert(header::REFERER, origin);
    }
    safe
}

pub(crate) async fn send_media(
    client: &Client,
    builder: reqwest::RequestBuilder,
    cancel: &CancellationToken,
) -> Result<Response> {
    send_observed(client, builder, cancel, &mut 0, true, |_, _, _| {}).await
}

async fn send(
    client: &Client,
    builder: reqwest::RequestBuilder,
    cancel: &CancellationToken,
) -> Result<Response> {
    send_observed(client, builder, cancel, &mut 0, false, |_, _, _| {}).await
}

async fn send_observed(
    client: &Client,
    builder: reqwest::RequestBuilder,
    cancel: &CancellationToken,
    retries: &mut u32,
    media: bool,
    mut observe: impl FnMut(&reqwest::Url, Option<&Response>, u8),
) -> Result<Response> {
    let mut request = builder.build().map_err(network)?;
    for redirects in 0..=10 {
        let previous = request.url().clone();
        let mut next_request = request.try_clone().ok_or(Error::InvalidHttpResponse)?;
        observe(&previous, None, redirects);
        let response = send_with_retries(client, &request, cancel, retries).await?;
        observe(response.url(), Some(&response), redirects);
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
            let mut safe = if media {
                media_headers(next_request.headers(), &previous, &next)
            } else {
                HeaderMap::new()
            };
            for name in [
                header::RANGE,
                header::IF_RANGE,
                header::USER_AGENT,
                header::ACCEPT_ENCODING,
            ] {
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
    retries: &mut u32,
) -> Result<Response> {
    for attempt in 0..3 {
        if attempt > 0 {
            *retries += 1;
        }
        let outgoing = request.try_clone().ok_or(Error::InvalidHttpResponse)?;
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            response = client.execute(outgoing) => response,
        };
        let backoff = Duration::from_millis(200 * (1 << attempt));
        let delay = match response {
            Ok(response)
                if matches!(response.status().as_u16(), 429 | 500 | 502 | 503 | 504)
                    && attempt < 2 =>
            {
                let delay = retry_after(response.headers(), std::time::SystemTime::now())
                    .unwrap_or_default()
                    .max(backoff);
                // Beyond this budget, surface the HTTP status and keep the saved parts.
                if delay > Duration::from_secs(60) {
                    return Ok(response);
                }
                delay
            }
            Ok(response) => return Ok(response),
            Err(_) if attempt < 2 => backoff,
            Err(error) => return Err(network(error)),
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            _ = tokio::time::sleep(delay) => {}
        }
    }
    Err(Error::InvalidHttpResponse)
}

fn retry_after(headers: &HeaderMap, now: std::time::SystemTime) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(Duration::from_secs(value.parse().unwrap_or(u64::MAX)));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(date.duration_since(now).unwrap_or_default())
}

fn network(error: reqwest::Error) -> Error {
    Error::Network(error.without_url())
}

// A strong ETag identifies a representation within one resource. Strip only
// recognized CDN signing fields after a redirect. Preserve the origin, path
// and raw resource selectors (including encoding, order and duplicates).
fn range_resource_key(redirected: bool, final_url: &reqwest::Url) -> String {
    let raw = final_url.as_str();
    let Some((base, query)) = raw.split_once('?') else {
        return raw.to_owned();
    };
    if !redirected {
        return raw.to_owned();
    }
    let (query, fragment) = query
        .split_once('#')
        .map_or((query, None), |(q, f)| (q, Some(f)));
    let key = |field: &str| {
        field
            .split('=')
            .next()
            .unwrap_or(field)
            .to_ascii_lowercase()
    };
    let signature = |k: &str| {
        matches!(
            k,
            "sig"
                | "signature"
                | "sign"
                | "token"
                | "jwt"
                | "auth_key"
                | "x-amz-signature"
                | "x-goog-signature"
        )
    };
    if !query.split('&').any(|field| signature(&key(field))) {
        return raw.to_owned();
    }
    let selectors: Vec<_> = query
        .split('&')
        .filter(|field| {
            let k = key(field);
            !signature(&k)
                && !matches!(
                    k.as_str(),
                    "expires"
                        | "policy"
                        | "key-pair-id"
                        | "se"
                        | "st"
                        | "x-amz-date"
                        | "x-amz-credential"
                        | "x-amz-security-token"
                        | "x-goog-date"
                        | "x-goog-credential"
                )
        })
        .collect();
    let mut result = format!("signed:{base}");
    if !selectors.is_empty() {
        result.push('?');
        result.push_str(&selectors.join("&"));
    }
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(fragment);
    }
    result
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

struct RangeSource<'a> {
    client: &'a Client,
    url: &'a str,
    headers: &'a HeaderMap,
    manifest: &'a ResumeManifest,
    limiter: &'a RateLimiter,
    cancel: &'a CancellationToken,
}

enum RangeOutcome {
    Complete,
    RateLimited,
}

impl RangeSource<'_> {
    async fn download(
        &self,
        directory: &Path,
        segments: &[Arc<SegmentState>],
        connections: usize,
        trace: &mut HttpTrace<'_>,
    ) -> Result<()> {
        let mut pending: VecDeque<_> = (0..segments.len()).collect();
        let mut transfers = FuturesUnordered::new();
        let mut limit = connections.min(segments.len());
        let mut rate_limited = None;
        let mut first_error = None;
        let mut completed_request = false;
        loop {
            while !self.cancel.is_cancelled()
                && transfers.len() < limit
                && let Some(index) = pending.pop_front()
            {
                let part = directory.join(format!("segment-{index}.part"));
                let range = self.manifest.bounds(index);
                let mut segment_trace = trace.clone();
                segment_trace.details.stage = HttpStage::Segment;
                segment_trace.details.http_status = None;
                transfers.push(async move {
                    segment_trace.retries = 0;
                    let mut issue = SegmentFailure {
                        index,
                        start: range.start,
                        end: range.end - 1,
                        expected_total: self.manifest.total,
                        response_range: None,
                        content_length: None,
                        strong_etag: false,
                        etag_matches: None,
                        final_url_matches: None,
                        effective_proxy: None,
                        retries: 0,
                        redirects: 0,
                        http_status: None,
                        stage: HttpStage::Segment,
                        issue: SegmentIssue::Request,
                        error: DownloadFailure::DownloadFailed,
                    };
                    let result = self
                        .fetch(
                            part,
                            range,
                            &segments[index],
                            &mut segment_trace,
                            &mut issue,
                        )
                        .await;
                    if !matches!(result, Ok(RangeOutcome::Complete)) {
                        issue.retries = segment_trace.retries;
                        issue.http_status = segment_trace.details.http_status;
                        issue.redirects = segment_trace.details.redirect_count;
                        issue.stage = segment_trace.details.stage;
                        issue.effective_proxy = segment_trace.details.effective_proxy;
                        issue.error = result
                            .as_ref()
                            .err()
                            .map_or(DownloadFailure::HttpStatus, DownloadFailure::from_error);
                        segment_trace.details.segment_failure = Some(Box::new(issue));
                    }
                    (index, result, segment_trace.details)
                });
            }
            let Some((index, result, details)) = transfers.next().await else {
                break;
            };
            if first_error.is_some() {
                continue; // Drain cancelled writers before fallback, assembly or publication.
            }
            match result {
                Ok(RangeOutcome::Complete) => {
                    if details.http_status.is_some() {
                        trace.details = details;
                        completed_request = true;
                    }
                }
                Ok(RangeOutcome::RateLimited) if limit != 1 => {
                    // Hold new requests until all current ranges settle. If at
                    // least one succeeded, finish the remaining queue serially.
                    limit = 0;
                    rate_limited = Some(details);
                    pending.push_back(index);
                }
                Ok(RangeOutcome::RateLimited) => {
                    first_error = Some((Error::HttpStatus(429), details))
                }
                Err(error) => first_error = Some((error, details)),
            }
            if transfers.is_empty()
                && let Some(details) = rate_limited.take()
            {
                if completed_request {
                    limit = 1;
                    trace.details.range_mode = Some(HttpRangeMode::RateLimited);
                    trace.record();
                } else if first_error.is_none() {
                    first_error = Some((Error::HttpStatus(429), details));
                }
            }
            if first_error.is_some() {
                self.cancel.cancel();
            }
        }
        if let Some((error, details)) = first_error {
            trace.details = details;
            trace.record();
            return Err(error);
        }
        Ok(())
    }

    async fn fetch(
        &self,
        part: PathBuf,
        range: std::ops::Range<u64>,
        segment: &Arc<SegmentState>,
        trace: &mut HttpTrace<'_>,
        issue: &mut SegmentFailure,
    ) -> Result<RangeOutcome> {
        let progress = trace.progress;
        let length = range.end - range.start;
        let end = range.end - 1;
        // One worker owns each part. Open it for writing so Windows can also
        // truncate an oversized response back to the last validated offset.
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&part)
            .await?;
        let mut present = file.seek(SeekFrom::End(0)).await?;
        if present > length {
            file.set_len(0).await?;
            file.rewind().await?;
            present = 0;
        }
        if present != segment.downloaded.load(Ordering::Relaxed) {
            issue.issue = SegmentIssue::LocalOffset;
            return Err(Error::InvalidHttpResponse);
        }
        if present == length {
            file.sync_data().await?;
            return Ok(RangeOutcome::Complete);
        }
        let _active = SegmentState::activate(segment);
        for attempt in 0..3 {
            if present == length {
                file.sync_data().await?;
                return Ok(RangeOutcome::Complete);
            }
            let range_start = range.start + present;
            issue.start = range_start;
            issue.response_range = None;
            issue.content_length = None;
            issue.strong_etag = false;
            issue.etag_matches = None;
            issue.final_url_matches = None;
            if attempt > 0 {
                trace.retries += 1;
            }
            let mut request = self
                .client
                .get(self.url)
                .headers(self.headers.clone())
                .header(header::RANGE, format!("bytes={range_start}-{end}"));
            if let RangeValidator::Etag(etag) = &self.manifest.validator {
                request = request.header(header::IF_RANGE, etag);
            }
            let response = trace
                .send(HttpStage::Segment, self.client, request, self.cancel)
                .await?;
            issue.response_range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range);
            issue.content_length = response
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok());
            let etag = strong_etag(response.headers());
            issue.strong_etag = etag.is_some();
            if let RangeValidator::Etag(expected) = &self.manifest.validator {
                issue.etag_matches = Some(etag.as_ref() == Some(expected));
                issue.final_url_matches = Some(
                    range_resource_key(trace.details.redirect_count > 0, response.url())
                        == self.manifest.resource_url,
                );
            }
            if response.status() == StatusCode::TOO_MANY_REQUESTS
                && !response.headers().contains_key(header::RETRY_AFTER)
            {
                issue.issue = SegmentIssue::HttpStatus;
                return Ok(RangeOutcome::RateLimited);
            }
            if response.status() != StatusCode::PARTIAL_CONTENT {
                if response.status() != StatusCode::OK {
                    issue.issue = SegmentIssue::HttpStatus;
                    return Err(Error::HttpStatus(response.status().as_u16()));
                }
                issue.issue = SegmentIssue::RangeIgnored;
                return Err(Error::InvalidHttpResponse);
            }
            let mismatch = if issue.final_url_matches == Some(false) {
                Some(SegmentIssue::ResourceChanged)
            } else if issue.etag_matches == Some(false) {
                Some(SegmentIssue::EtagMismatch)
            } else if issue.response_range != Some((range_start, end, self.manifest.total)) {
                Some(SegmentIssue::ContentRangeMismatch)
            } else {
                None
            };
            if let Some(reason) = mismatch {
                issue.issue = reason;
                trace.stage(HttpStage::IntegrityCheck);
                return Err(Error::InvalidHttpResponse);
            }
            let result = write_response(
                response,
                &mut file,
                self.limiter,
                self.cancel,
                progress,
                Some(segment),
                BodyIntegrity {
                    maximum: Some(length - present),
                    sha256: None,
                },
            )
            .await;
            file.flush().await?;
            match result {
                Ok(written) if written == length - present => {
                    file.sync_data().await?;
                    return Ok(RangeOutcome::Complete);
                }
                // A chunked response can end early with clean EOF. The same validated
                // prefix can be resumed after either clean truncation or a socket error.
                Ok(_) | Err(Error::Network(_)) if attempt < 2 => {
                    let next_present = file.metadata().await?.len();
                    if next_present < present || next_present > length {
                        return Err(Error::InvalidHttpResponse);
                    }
                    present = next_present;
                    tokio::select! {
                        biased;
                        _ = self.cancel.cancelled() => return Err(Error::Cancelled),
                        _ = tokio::time::sleep(Duration::from_millis(200 * (1 << attempt))) => {}
                    }
                }
                Ok(_) => {
                    issue.issue = SegmentIssue::ShortBody;
                    trace.stage(HttpStage::IntegrityCheck);
                    return Err(Error::InvalidHttpResponse);
                }
                Err(Error::InvalidHttpResponse) => {
                    issue.issue = SegmentIssue::OversizedBody;
                    // The body exceeded its declared range. Its earlier chunks
                    // must not become a trusted prefix on the next resume.
                    file.set_len(present).await?;
                    file.sync_data().await?;
                    let discarded = segment.downloaded.swap(present, Ordering::Relaxed) - present;
                    progress
                        .0
                        .downloaded
                        .fetch_sub(discarded, Ordering::Relaxed);
                    trace.stage(HttpStage::IntegrityCheck);
                    return Err(Error::InvalidHttpResponse);
                }
                Err(error) => return Err(error),
            }
        }
        Err(Error::InvalidHttpResponse)
    }
}

struct BodyIntegrity {
    maximum: Option<u64>,
    sha256: Option<Sha256Bytes>,
}

async fn write_response(
    response: Response,
    file: &mut fs::File,
    limiter: &RateLimiter,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
    segment: Option<&SegmentState>,
    integrity: BodyIntegrity,
) -> Result<u64> {
    let mut body = response.bytes_stream();
    let mut total = 0u64;
    let mut hasher = integrity.sha256.map(|_| Sha256::new());
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
        if integrity.maximum.is_some_and(|maximum| next > maximum) {
            return Err(Error::InvalidHttpResponse);
        }
        limiter.wait(chunk.len(), cancel).await?;
        file.write_all(&chunk).await?;
        if let Some(h) = &mut hasher {
            h.update(&chunk);
        }
        progress.add(chunk.len() as u64);
        if let Some(segment) = segment {
            segment
                .downloaded
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        total = next;
    }
    if let (Some(h), Some(expected)) = (hasher, integrity.sha256)
        && <Sha256Bytes>::from(h.finalize()) != expected
    {
        return Err(Error::ChecksumMismatch);
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

fn response_basename(response: &Response) -> Option<String> {
    response
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

pub(crate) fn is_segment_filename(name: &str) -> bool {
    name.strip_prefix("segment-")
        .and_then(|name| name.strip_suffix(".part"))
        .and_then(|index| index.parse::<usize>().ok())
        .is_some_and(|index| index < MAX_PARTS && name == format!("segment-{index}.part"))
}

async fn clear_parts(work_dir: &Path) -> Result<()> {
    let mut entries = fs::read_dir(work_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name().to_str().is_some_and(is_segment_filename) {
            remove_if_present(entry.path()).await?;
        }
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
    fn interprets_retry_after_delays_and_http_dates() {
        let now = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap();
        for (value, expected) in [
            ("5", Some(5)),
            ("Sun, 06 Nov 1994 08:49:45 GMT", Some(8)),
            ("Sun, 06 Nov 1994 08:49:30 GMT", Some(0)),
            ("99999999999999999999", Some(u64::MAX)),
            ("invalid", None),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::RETRY_AFTER, HeaderValue::from_static(value));
            assert_eq!(
                retry_after(&headers, now),
                expected.map(Duration::from_secs)
            );
        }
    }

    #[test]
    fn range_plan_matches_size_workers_and_probe_latency() {
        let mib = 1024 * 1024;
        assert_eq!(part_count(16, 16, Duration::ZERO), 1);
        assert_eq!(part_count(16 * mib, 1, Duration::ZERO), 1);
        assert_eq!(part_count(16 * mib, 4, Duration::ZERO), 8);
        assert_eq!(part_count(32 * mib, 4, Duration::ZERO), 16);
        assert_eq!(part_count(16 * mib, 4, Duration::from_millis(75)), 4);
        assert_eq!(part_count(u64::MAX, 16, Duration::from_secs(60)), MAX_PARTS);
        for index in [0, 15, 16, MAX_PARTS - 1] {
            assert!(is_segment_filename(&format!("segment-{index}.part")));
        }
        assert!(!is_segment_filename(&format!("segment-{MAX_PARTS}.part")));
        assert!(!is_segment_filename("segment-00.part"));
    }

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
            &ResumeManifest {
                parts: 2,
                total: 12,
                validator: RangeValidator::Etag(String::new()),
                resource_url: String::new(),
            },
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
                None,
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
            &ResumeManifest {
                parts: 1,
                total: 14,
                validator: RangeValidator::Etag(String::new()),
                resource_url: String::new(),
            },
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

#[cfg(test)]
mod route_tests {
    use super::*;
    use crate::TunEgress;
    use axum::{Router, routing::get};

    #[tokio::test]
    async fn os_route_and_tun_exit_use_distinct_transports_without_fallback() {
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/download", origin.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(origin, Router::new().fallback(get(|| async { "origin" })))
                .await
                .unwrap();
        });
        let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = local.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(
                local,
                Router::new().fallback(get(|uri: axum::http::Uri| async move {
                    assert!(uri.authority().is_some());
                    "local listener"
                })),
            )
            .await
            .unwrap();
        });
        let application_proxy = ProxyOptions {
            url: "http://127.0.0.1:0".into(),
            username: None,
            password: None,
        };
        let client = |mode, exit: Option<&TunEgress>| {
            network_client(Some(&application_proxy), mode, None, exit)
        };
        let local = TunEgress::parse(&port.to_string()).unwrap();
        for (mode, expected) in [
            (ProxyMode::SystemRoute, "origin"),
            (ProxyMode::TunDirect, "local listener"),
        ] {
            assert_eq!(
                client(mode, Some(&local))
                    .unwrap()
                    .get(&url)
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                expected
            );
        }
        assert!(matches!(
            client(ProxyMode::TunDirect, None),
            Err(Error::TunEgressUnavailable)
        ));
        assert!(TunEgress::parse("0").is_err());
        assert!(TunEgress::parse("65536").is_err());
        assert!(TunEgress::parse("https://example.org").is_err());
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let nonexistent = TunEgress::parse("ndm-missing-if").unwrap();
            assert!(
                client(ProxyMode::TunDirect, Some(&nonexistent))
                    .unwrap()
                    .get(&url)
                    .send()
                    .await
                    .is_err()
            );
            let loopback = TunEgress::parse(if cfg!(target_os = "macos") {
                "lo0"
            } else {
                "lo"
            })
            .unwrap();
            assert_eq!(
                client(ProxyMode::TunDirect, Some(&loopback))
                    .unwrap()
                    .get(&url)
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "origin"
            );
        }
    }
}
