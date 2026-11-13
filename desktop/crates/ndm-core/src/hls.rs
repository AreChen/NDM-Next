use std::{
    fmt,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{StreamExt, future::try_join_all};
use hls_engine::{
    ByteRange, EmbeddedAudio, EngineEndReason, EngineInput, EngineInputs, EngineMode,
    EngineOptions, EngineSession, EngineWait, FileOutputOptions, OutputFormat, Source,
    SourceLocation, TextResource, TrackMetadata,
    crypto::key::{
        AvailableKey, KeyClock, KeyFuture, KeyProvider, KeyRequest, KeyResolution, KeySession,
        KeySessionOptions, SecretKey,
    },
    playlist::{EncryptionMethod, InputId, PlaylistContext, parse_playlist_snapshot},
};
use m3u8_rs::{AlternativeMediaType, Playlist};
use reqwest::{
    Client, StatusCode, Url,
    header::{self, HeaderMap},
};
use serde::{Deserialize, Serialize};
use tokio::{fs, io::AsyncReadExt};
use tokio_util::sync::CancellationToken;

use crate::{
    DownloadProgress, Error, NewTask, ProxyOptions, Result, Store, TaskId,
    http::{
        RateLimiter, network_client, parse_content_range, publish_no_overwrite, request_headers,
        safe_filename, send,
    },
};

const MAX_PLAYLIST_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESOURCE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HlsOptions {
    /// Zero-based index among playable variants; defaults to the highest bandwidth.
    pub variant_index: Option<usize>,
    /// Names in the chosen variant's audio group; defaults to its default track.
    pub audio_names: Vec<String>,
    pub max_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub proxy: Option<ProxyOptions>,
}

#[derive(Clone, Debug)]
pub struct HlsOutcome {
    pub path: PathBuf,
    pub tracks: usize,
    pub stopped: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HlsAudio {
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HlsVariant {
    pub index: usize,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
    pub audio_tracks: Vec<HlsAudio>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HlsCatalog {
    pub direct_media: bool,
    pub variants: Vec<HlsVariant>,
}

#[derive(Clone)]
struct NetworkSource {
    client: Client,
    origin: Url,
    headers: HeaderMap,
    cancel: CancellationToken,
    limiter: Arc<RateLimiter>,
    progress: DownloadProgress,
}

impl fmt::Debug for NetworkSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetworkSource").finish_non_exhaustive()
    }
}

impl NetworkSource {
    async fn fetch(
        &self,
        location: &SourceLocation,
        range: Option<&ByteRange>,
        limit: usize,
    ) -> hls_engine::SourceResult<(Vec<u8>, Url)> {
        let SourceLocation::Url(url) = location else {
            return Err(hls_engine::Error::unsupported(
                "local resources are not allowed",
            ));
        };
        if !matches!(url.scheme(), "http" | "https") {
            return Err(hls_engine::Error::unsupported(
                "unsupported resource scheme",
            ));
        }
        let mut builder = self.client.get(url.clone());
        if url.origin() == self.origin.origin() {
            builder = builder.headers(self.headers.clone());
        }
        if let Some(range) = range {
            let end = range
                .offset
                .checked_add(range.length)
                .and_then(|end| end.checked_sub(1))
                .ok_or_else(|| hls_engine::Error::invalid("invalid resource range"))?;
            builder = builder.header(header::RANGE, format!("bytes={}-{end}", range.offset));
        }
        let response = send(&self.client, builder, &self.cancel)
            .await
            .map_err(|_| hls_engine::Error::Http("resource request failed".into()))?;
        if let Some(range) = range {
            let expected_end = range.offset + range.length - 1;
            let returned = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range);
            if response.status() != StatusCode::PARTIAL_CONTENT
                || !matches!(returned, Some((start, end, total)) if start == range.offset && end == expected_end && end < total)
            {
                return Err(hls_engine::Error::invalid(
                    "invalid resource range response",
                ));
            }
        } else if !response.status().is_success() {
            return Err(hls_engine::Error::Http("resource rejected".into()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err(hls_engine::Error::invalid("resource exceeds size limit"));
        }
        let final_url = response.url().clone();
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::select! {
            _ = self.cancel.cancelled() => return Err(hls_engine::Error::Cancelled),
            chunk = stream.next() => chunk,
        } {
            let chunk =
                chunk.map_err(|_| hls_engine::Error::Http("resource body failed".into()))?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(hls_engine::Error::invalid("resource exceeds size limit"));
            }
            self.limiter
                .wait(chunk.len(), &self.cancel)
                .await
                .map_err(|_| hls_engine::Error::Cancelled)?;
            self.progress.add(chunk.len() as u64);
            bytes.extend_from_slice(&chunk);
        }
        if range.is_some_and(|range| bytes.len() as u64 != range.length) {
            return Err(hls_engine::Error::invalid("short resource range"));
        }
        Ok((bytes, final_url))
    }
}

impl Source for NetworkSource {
    fn read_text<'a>(
        &'a self,
        location: &'a SourceLocation,
    ) -> Pin<Box<dyn Future<Output = hls_engine::SourceResult<TextResource>> + Send + 'a>> {
        Box::pin(async move {
            let (bytes, final_url) = self.fetch(location, None, MAX_PLAYLIST_BYTES).await?;
            let content = String::from_utf8(bytes)
                .map_err(|_| hls_engine::Error::invalid("playlist is not UTF-8"))?;
            Ok(TextResource {
                content,
                location: SourceLocation::Url(final_url),
            })
        })
    }

    fn read_bytes<'a>(
        &'a self,
        location: &'a SourceLocation,
        range: Option<&'a ByteRange>,
    ) -> Pin<Box<dyn Future<Output = hls_engine::SourceResult<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move {
            self.fetch(location, range, MAX_RESOURCE_BYTES)
                .await
                .map(|(bytes, _)| bytes)
        })
    }
}

struct Keys(Arc<NetworkSource>);
impl KeyProvider for Keys {
    fn resolve(&self, request: KeyRequest) -> KeyFuture<KeyResolution> {
        let source = self.0.clone();
        Box::pin(async move {
            if request.reference().method() != &EncryptionMethod::Aes128
                || request.reference().format() != "identity"
            {
                return KeyResolution::Unavailable;
            }
            let location = request.reference().location().location().clone();
            let result = tokio::select! {
                _ = request.cancellation().cancelled() => return KeyResolution::Unavailable,
                result = source.fetch(&location, None, 16) => result,
            };
            match result
                .ok()
                .and_then(|(bytes, _)| SecretKey::new(bytes).ok())
            {
                Some(key) => KeyResolution::Available(AvailableKey::aes128(key)),
                None => KeyResolution::Unavailable,
            }
        })
    }
}

struct Clock(Instant);
impl KeyClock for Clock {
    fn now(&self) -> u64 {
        self.0.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
    }
}
struct Wait;
impl EngineWait for Wait {
    fn wait(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

fn network_source(
    request: &NewTask,
    cancel: CancellationToken,
    max_bytes_per_second: Option<u64>,
    proxy: Option<&ProxyOptions>,
    progress: DownloadProgress,
) -> Result<(Arc<NetworkSource>, Url)> {
    let url = Url::parse(&request.url).map_err(|_| Error::Hls)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::Hls);
    }
    let client = network_client(proxy)?;
    let source = Arc::new(NetworkSource {
        client,
        origin: url.clone(),
        headers: request_headers(request)?,
        cancel,
        limiter: Arc::new(RateLimiter::new(max_bytes_per_second)),
        progress,
    });
    Ok((source, url))
}

/// Retrieve the available quality and external audio choices without downloading media.
pub async fn inspect_hls(request: &NewTask, cancel: CancellationToken) -> Result<HlsCatalog> {
    inspect_hls_with_options(request, &HlsOptions::default(), cancel).await
}

pub async fn inspect_hls_with_options(
    request: &NewTask,
    options: &HlsOptions,
    cancel: CancellationToken,
) -> Result<HlsCatalog> {
    let (source, url) = network_source(
        request,
        cancel.clone(),
        None,
        options.proxy.as_ref(),
        DownloadProgress::default(),
    )?;
    let text = source
        .read_text(&SourceLocation::Url(url))
        .await
        .map_err(|_| {
            if cancel.is_cancelled() {
                Error::Cancelled
            } else {
                Error::Hls
            }
        })?;
    match m3u8_rs::parse_playlist_res(text.content.as_bytes()).map_err(|_| Error::Hls)? {
        Playlist::MediaPlaylist(_) => Ok(HlsCatalog {
            direct_media: true,
            variants: Vec::new(),
        }),
        Playlist::MasterPlaylist(master) => {
            if !master.session_key.is_empty() {
                return Err(Error::Hls);
            }
            let variants = master
                .variants
                .iter()
                .filter(|variant| !variant.is_i_frame)
                .enumerate()
                .map(|(index, variant)| HlsVariant {
                    index,
                    bandwidth: variant.bandwidth,
                    resolution: variant.resolution.as_ref().map(|r| (r.width, r.height)),
                    audio_tracks: master
                        .alternatives
                        .iter()
                        .filter(|track| {
                            track.media_type == AlternativeMediaType::Audio
                                && variant.audio.as_ref() == Some(&track.group_id)
                                && track.uri.is_some()
                        })
                        .map(|track| HlsAudio {
                            name: track.name.clone(),
                            language: track.language.clone(),
                            default: track.default,
                        })
                        .collect(),
                })
                .collect::<Vec<_>>();
            if variants.is_empty() {
                return Err(Error::Hls);
            }
            Ok(HlsCatalog {
                direct_media: false,
                variants,
            })
        }
    }
}

pub async fn execute_hls(
    store: &Store,
    id: TaskId,
    options: &HlsOptions,
    cancel: CancellationToken,
    stop: CancellationToken,
    progress: DownloadProgress,
) -> Result<HlsOutcome> {
    let task = store.get(id)?.ok_or(Error::NotFound)?;
    store.start(id)?;
    match download_hls_with_progress(&task.request, id, options, cancel, stop, progress).await {
        Ok(outcome) => {
            store.complete(id, &outcome.path, false)?;
            Ok(outcome)
        }
        Err(Error::Cancelled) => {
            store.pause(id)?;
            Err(Error::Cancelled)
        }
        Err(error) => {
            store.fail(id, "HLS media or playlist failed")?;
            Err(error)
        }
    }
}

pub async fn download_hls(
    request: &NewTask,
    id: TaskId,
    options: &HlsOptions,
    cancel: CancellationToken,
    stop: CancellationToken,
) -> Result<HlsOutcome> {
    download_hls_with_progress(
        request,
        id,
        options,
        cancel,
        stop,
        DownloadProgress::default(),
    )
    .await
}

async fn download_hls_with_progress(
    request: &NewTask,
    id: TaskId,
    options: &HlsOptions,
    cancel: CancellationToken,
    stop: CancellationToken,
    progress: DownloadProgress,
) -> Result<HlsOutcome> {
    let (source, url) = network_source(
        request,
        cancel.clone(),
        options.max_bytes_per_second,
        options.proxy.as_ref(),
        progress,
    )?;
    let first = source
        .read_text(&SourceLocation::Url(url))
        .await
        .map_err(|_| {
            if cancel.is_cancelled() {
                Error::Cancelled
            } else {
                Error::Hls
            }
        })?;
    let base = match &first.location {
        SourceLocation::Url(url) => url,
        _ => return Err(Error::Hls),
    };
    let parsed = m3u8_rs::parse_playlist_res(first.content.as_bytes()).map_err(|_| Error::Hls)?;
    let primary = InputId::new("primary").map_err(|_| Error::Hls)?;
    let mut selected = Vec::new();
    let mut external_audio = Vec::new();
    let primary_location = match parsed {
        Playlist::MediaPlaylist(_) => {
            if options.variant_index.is_some() || !options.audio_names.is_empty() {
                return Err(Error::Hls);
            }
            first.location.clone()
        }
        Playlist::MasterPlaylist(master) => {
            if !master.session_key.is_empty() {
                return Err(Error::Hls);
            }
            let variants: Vec<_> = master.variants.iter().filter(|v| !v.is_i_frame).collect();
            let variant = if let Some(index) = options.variant_index {
                *variants.get(index).ok_or(Error::Hls)?
            } else {
                *variants
                    .iter()
                    .max_by_key(|v| v.bandwidth)
                    .ok_or(Error::Hls)?
            };
            if let Some(group) = &variant.audio {
                let candidates: Vec<_> = master
                    .alternatives
                    .iter()
                    .filter(|a| {
                        a.media_type == AlternativeMediaType::Audio
                            && &a.group_id == group
                            && a.uri.is_some()
                    })
                    .collect();
                if options.audio_names.is_empty() {
                    if let Some(track) = candidates
                        .iter()
                        .find(|a| a.default)
                        .or_else(|| candidates.first())
                    {
                        external_audio.push((*track).clone());
                    }
                } else {
                    for name in &options.audio_names {
                        let track = candidates
                            .iter()
                            .find(|a| &a.name == name)
                            .ok_or(Error::Hls)?;
                        if external_audio
                            .iter()
                            .any(|existing: &m3u8_rs::AlternativeMedia| existing.name == track.name)
                        {
                            return Err(Error::Hls);
                        }
                        external_audio.push((*track).clone());
                    }
                }
            } else if !options.audio_names.is_empty() {
                return Err(Error::Hls);
            }
            SourceLocation::Url(base.join(&variant.uri).map_err(|_| Error::Hls)?)
        }
    };
    let mut inputs = EngineInputs::new(
        EngineInput::new(primary.clone(), source.clone()),
        if external_audio.is_empty() {
            EmbeddedAudio::Keep
        } else {
            EmbeddedAudio::Exclude
        },
    );
    selected.push((primary, primary_location));
    for (index, audio) in external_audio.iter().enumerate() {
        let input_id = InputId::new(format!("audio-{index}")).map_err(|_| Error::Hls)?;
        let location = base
            .join(audio.uri.as_deref().ok_or(Error::Hls)?)
            .map_err(|_| Error::Hls)?;
        let metadata = TrackMetadata::new(audio.language.as_deref().unwrap_or("und"), &audio.name)
            .with_default(audio.default);
        inputs = inputs.with_audio(EngineInput::new(input_id.clone(), source.clone()), metadata);
        selected.push((input_id, SourceLocation::Url(location)));
    }
    let keys = KeySession::new(
        id.to_string(),
        "hls",
        Arc::new(Keys(source.clone())),
        Arc::new(Clock(Instant::now())),
        KeySessionOptions::default(),
    )
    .map_err(|_| Error::Hls)?;
    let engine = EngineSession::new(
        inputs,
        keys,
        EngineOptions::default()
            .with_mode(EngineMode::Open)
            .with_download_concurrency(4)
            .with_waiter(Arc::new(Wait), Duration::from_secs(30)),
    )
    .map_err(|_| Error::Hls)?;
    let handle = engine.handle();
    fs::create_dir_all(&request.destination).await?;
    let work_dir = request.destination.join(format!(".ndm-next-{id}"));
    fs::create_dir_all(&work_dir).await?;
    let temporary = work_dir.join("hls-output.mp4");
    let producer = async {
        let jobs = selected.into_iter().map(|(input_id, location)| {
            let handle = handle.clone();
            let source = source.clone();
            let cancel = cancel.clone();
            let stop = stop.clone();
            async move {
                let mut revision = 0u64;
                loop {
                    let text = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => { handle.cancel(); return Err(Error::Cancelled); }
                        _ = stop.cancelled() => { handle.stop(); return Ok(()); }
                        text = source.read_text(&location) => text.map_err(|_| Error::Hls)?,
                    };
                    let snapshot = parse_playlist_snapshot(&text, PlaylistContext::new(input_id.clone(), 0).with_revision(revision))
                        .map_err(|_| Error::Hls)?;
                    let end = snapshot.end_list();
                    let delay = Duration::from_secs(snapshot.target_duration().unwrap_or(2).clamp(1, 30));
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => { handle.cancel(); return Err(Error::Cancelled); }
                        _ = stop.cancelled() => { handle.stop(); return Ok(()); }
                        accepted = handle.accept_when_ready(&input_id, &snapshot) => accepted.map_err(|_| Error::Hls)?,
                    };
                    if end { return Ok(()); }
                    revision = revision.checked_add(1).ok_or(Error::Hls)?;
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => { handle.cancel(); return Err(Error::Cancelled); }
                        _ = stop.cancelled() => { handle.stop(); return Ok(()); }
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
            }
        });
        try_join_all(jobs).await.map(|_| ())
    };
    let consumer = async {
        engine
            .write_to_file(
                &temporary,
                FileOutputOptions::default().with_format(OutputFormat::FragmentedMp4),
            )
            .await
            .map_err(|_| {
                if cancel.is_cancelled() {
                    Error::Cancelled
                } else {
                    Error::Hls
                }
            })
    };
    let (_, report) = tokio::try_join!(producer, consumer)?;
    if report.media().bytes_written() == 0 || report.tracks().is_empty() {
        return Err(Error::Hls);
    }
    let mut file = fs::File::open(&temporary).await?;
    let mut header = [0u8; 8];
    file.read_exact(&mut header).await?;
    if &header[4..8] != b"ftyp" {
        return Err(Error::Hls);
    }
    drop(file);
    let suggested = safe_filename(request.suggested_filename.as_deref().unwrap_or("video.mp4"));
    let path = std::path::Path::new(&suggested);
    let name = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
    {
        suggested
    } else {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("video");
        format!("{stem}.mp4")
    };
    let target = publish_no_overwrite(&temporary, &request.destination, &name).await?;
    let _ = fs::remove_file(&temporary).await;
    let _ = fs::remove_dir(&work_dir).await;
    Ok(HlsOutcome {
        path: target,
        tracks: report.tracks().len(),
        stopped: report.media().end_reason() == EngineEndReason::Stop,
    })
}
