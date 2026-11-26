//! Bounded static ISO-BMFF DASH. All networking is performed here with scoped
//! credentials; only complete local tracks are passed to the media tools.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_util::StreamExt;
use reqwest::{
    Client, StatusCode, Url,
    header::{self, HeaderMap},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{
    DownloadProgress, Error, NewTask, ProxyOptions, Result, Store, TaskId,
    http::{
        RateLimiter, media_headers, network_client, parse_content_range, publish_download,
        request_headers, safe_filename, send_media,
    },
    media::{LocalInput, LocalTools, remux_local},
};

mod parser;

const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SELECTED_AUDIO: usize = 8;
const MAX_CHECKPOINT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashOptions {
    /// Zero-based video representation; defaults to the highest bandwidth.
    pub representation_index: Option<usize>,
    /// Zero-based audio representations; defaults to the preferred audio track.
    pub audio_indices: Vec<usize>,
    pub max_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub proxy: Option<ProxyOptions>,
    #[serde(default)]
    pub proxy_mode: crate::ProxyMode,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tun_egress: Option<crate::TunEgress>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DashRepresentation {
    pub index: usize,
    pub id: String,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
    pub codecs: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DashAudio {
    pub index: usize,
    /// Renditions in one adaptation group are alternative bitrates of one track.
    pub group: usize,
    pub id: String,
    pub language: Option<String>,
    pub label: Option<String>,
    pub bandwidth: u64,
    pub codecs: Option<String>,
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DashCatalog {
    pub representations: Vec<DashRepresentation>,
    pub audio_tracks: Vec<DashAudio>,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct DashOutcome {
    pub path: PathBuf,
    pub tracks: usize,
}

struct Network {
    client: Client,
    origin: Url,
    headers: HeaderMap,
    cancel: CancellationToken,
    limiter: Arc<RateLimiter>,
    progress: DownloadProgress,
}

struct Fetched {
    bytes: Vec<u8>,
    url: Url,
    etag: Option<String>,
    unchanged: bool,
}

impl Network {
    fn new(
        request: &NewTask,
        options: &DashOptions,
        cancel: CancellationToken,
        progress: DownloadProgress,
    ) -> Result<Self> {
        let origin = Url::parse(&request.url).map_err(|_| Error::Dash)?;
        if !matches!(origin.scheme(), "http" | "https")
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
        {
            return Err(Error::Dash);
        }
        Ok(Self {
            client: network_client(
                options.proxy.as_ref(),
                options.proxy_mode,
                options.user_agent.as_deref(),
                options.tun_egress.as_ref(),
            )?,
            origin,
            headers: request_headers(request)?,
            cancel,
            limiter: Arc::new(RateLimiter::new(options.max_bytes_per_second)),
            progress,
        })
    }

    async fn fetch(
        &self,
        url: &Url,
        range: Option<(u64, u64)>,
        limit: usize,
        etag: Option<&str>,
    ) -> Result<Fetched> {
        let mut request =
            self.client
                .get(url.clone())
                .headers(media_headers(&self.headers, &self.origin, url));
        if let Some((start, end)) = range {
            if end.saturating_sub(start) >= limit as u64 {
                return Err(Error::Dash);
            }
            request = request.header(header::RANGE, format!("bytes={start}-{end}"));
        }
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let response = send_media(&self.client, request, &self.cancel).await?;
        if response.status() == StatusCode::NOT_MODIFIED && etag.is_some() {
            return Ok(Fetched {
                bytes: Vec::new(),
                url: response.url().clone(),
                etag: None,
                unchanged: true,
            });
        }
        if let Some((start, end)) = range {
            let returned = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range);
            if response.status() != StatusCode::PARTIAL_CONTENT
                || !matches!(returned, Some((a, b, total)) if a == start && b == end && b < total)
            {
                return Err(Error::Dash);
            }
        } else if response.status() != StatusCode::OK {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        if response
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_some_and(|value| value != "identity")
        {
            return Err(Error::Dash);
        }
        let length = response.content_length();
        if length.is_some_and(|length| length > limit as u64) {
            return Err(Error::Dash);
        }
        let final_url = response.url().clone();
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.starts_with('"') && value.ends_with('"') && value.len() <= 1024)
            .map(str::to_owned);
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(Error::Cancelled),
            chunk = stream.next() => chunk,
        } {
            let chunk = chunk.map_err(|error| Error::Network(error.without_url()))?;
            self.progress.received(chunk.len() as u64);
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(Error::Dash);
            }
            self.limiter.wait(chunk.len(), &self.cancel).await?;
            self.progress.add(chunk.len() as u64);
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty()
            || length.is_some_and(|length| length != bytes.len() as u64)
            || range.is_some_and(|(start, end)| bytes.len() as u64 != end - start + 1)
        {
            return Err(Error::Dash);
        }
        Ok(Fetched {
            bytes,
            url: final_url,
            etag,
            unchanged: false,
        })
    }

    async fn manifest(&self) -> Result<(parser::Manifest, Vec<u8>, Url)> {
        let fetched = self
            .fetch(&self.origin, None, MAX_MANIFEST_BYTES, None)
            .await?;
        let manifest = parser::parse(&fetched.bytes, &fetched.url)?;
        Ok((manifest, fetched.bytes, fetched.url))
    }
}

pub async fn inspect_dash(request: &NewTask, cancel: CancellationToken) -> Result<DashCatalog> {
    inspect_dash_with_options(request, &DashOptions::default(), cancel).await
}

pub async fn inspect_dash_with_options(
    request: &NewTask,
    options: &DashOptions,
    cancel: CancellationToken,
) -> Result<DashCatalog> {
    let network = Network::new(request, options, cancel, DownloadProgress::default())?;
    Ok(network.manifest().await?.0.catalog)
}

pub async fn download_dash(
    request: &NewTask,
    id: TaskId,
    options: &DashOptions,
    cancel: CancellationToken,
) -> Result<DashOutcome> {
    download_with_progress(
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

/// The directory must contain the trusted network-disabled FFmpeg/FFprobe build.
pub async fn download_dash_with_media_tool(
    request: &NewTask,
    id: TaskId,
    options: &DashOptions,
    cancel: CancellationToken,
    media_tool_dir: &Path,
) -> Result<DashOutcome> {
    download_with_progress(
        request,
        id,
        options,
        cancel,
        DownloadProgress::default(),
        Some(media_tool_dir),
        None,
    )
    .await
}

pub async fn execute_dash(
    store: &Store,
    id: TaskId,
    options: &DashOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
) -> Result<DashOutcome> {
    let task = store.get(id)?.ok_or(Error::NotFound)?;
    store.start(id)?;
    match download_with_progress(
        &task.request,
        id,
        options,
        cancel,
        progress,
        None,
        Some(&task.work_directory()),
    )
    .await
    {
        Ok(outcome) => {
            store.complete(id, &outcome.path, false)?;
            Ok(outcome)
        }
        Err(Error::Cancelled) => {
            store.pause(id)?;
            Err(Error::Cancelled)
        }
        Err(error) => {
            let reason = match error {
                Error::TunEgressUnavailable => {
                    crate::DownloadFailure::TunEgressUnavailable.message()
                }
                Error::DashDrmUnsupported => "dash_drm_unsupported",
                Error::DashDynamicUnsupported => "dash_dynamic_unsupported",
                Error::DashUnsupported => "dash_layout_unsupported",
                Error::DashRecovery => "dash_recovery_conflict",
                Error::MediaToolUnavailable => "media_tool_unavailable",
                _ => "dash_failed",
            };
            store.fail(id, reason)?;
            Err(error)
        }
    }
}

fn selected_tracks(
    manifest: &parser::Manifest,
    options: &DashOptions,
) -> Result<Vec<parser::Track>> {
    let mut tracks = Vec::new();
    if !manifest.video.is_empty() {
        let index = options
            .representation_index
            .or_else(|| {
                manifest
                    .catalog
                    .representations
                    .iter()
                    .max_by_key(|track| track.bandwidth)
                    .map(|track| track.index)
            })
            .ok_or(Error::Dash)?;
        tracks.push(manifest.video.get(index).ok_or(Error::Dash)?.clone());
    } else if options.representation_index.is_some() {
        return Err(Error::Dash);
    }
    let audio_indices = if options.audio_indices.is_empty() {
        manifest
            .catalog
            .audio_tracks
            .iter()
            .find(|track| track.default)
            .map(|track| vec![track.index])
            .unwrap_or_default()
    } else {
        options.audio_indices.clone()
    };
    if audio_indices.len() > MAX_SELECTED_AUDIO {
        return Err(Error::Dash);
    }
    for index in audio_indices {
        let track = manifest.audio.get(index).ok_or(Error::Dash)?;
        if tracks
            .iter()
            .any(|existing| existing.kind == "audio" && existing.group == track.group)
        {
            return Err(Error::Dash);
        }
        tracks.push(track.clone());
    }
    if tracks.is_empty() {
        return Err(Error::Dash);
    }
    Ok(tracks)
}

#[derive(Default, Deserialize, Serialize)]
struct Checkpoint {
    plan: String,
    #[serde(default)]
    source_digest: String,
    resources: BTreeMap<usize, Receipt>,
}

#[derive(Deserialize, Serialize)]
struct Receipt {
    etag: Option<String>,
    digest: String,
    length: u64,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

async fn regular_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() && !reparse(&metadata) => Ok(true),
        Ok(_) => Err(Error::DashRecovery),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn reparse(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

async fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    regular_file(path).await?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    Ok(())
}

async fn save_checkpoint(directory: &Path, checkpoint: &Checkpoint) -> Result<()> {
    let path = directory.join("dash-checkpoint.json");
    let temporary = directory.join("dash-checkpoint.tmp");
    regular_file(&path).await?;
    let bytes = serde_json::to_vec(checkpoint)?;
    if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
        return Err(Error::Dash);
    }
    write_file(&temporary, &bytes).await?;
    fs::rename(temporary, path).await?;
    Ok(())
}

pub(crate) fn owns_partial_name(name: &str) -> bool {
    matches!(
        name,
        "dash-checkpoint.json" | "dash-checkpoint.tmp" | "dash-output.mp4" | "dash-output.mkv"
    ) || name
        .strip_prefix("dash-resource-")
        .and_then(|name| name.strip_suffix(".part"))
        .and_then(|index| index.parse::<usize>().ok())
        .is_some_and(|index| index < 65_536 && name == format!("dash-resource-{index}.part"))
        || name
            .strip_prefix("dash-track-")
            .and_then(|name| name.strip_suffix(".mp4"))
            .and_then(|index| index.parse::<usize>().ok())
            .is_some_and(|index| index < 9 && name == format!("dash-track-{index}.mp4"))
}

async fn restart_owned_files(directory: &Path) -> Result<()> {
    let mut entries = fs::read_dir(directory).await?;
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if owns_partial_name(&entry.file_name().to_string_lossy()) {
            if !regular_file(&entry.path()).await? {
                return Err(Error::DashRecovery);
            }
            files.push(entry.path());
        }
    }
    // Validate every target before deleting any. Unknown names are preserved.
    for file in files {
        fs::remove_file(file).await?;
    }
    Ok(())
}

fn next_bmff_box<'a>(bytes: &mut &'a [u8], budget: &mut usize) -> Result<(&'a [u8], &'a [u8])> {
    if bytes.len() < 8 || *budget == 0 {
        return Err(Error::Dash);
    }
    *budget -= 1;
    let raw = u32::from_be_bytes(bytes[..4].try_into().map_err(|_| Error::Dash)?);
    let (size, header) = match raw {
        0 => (bytes.len(), 8),
        1 if bytes.len() >= 16 => (
            usize::try_from(u64::from_be_bytes(
                bytes[8..16].try_into().map_err(|_| Error::Dash)?,
            ))
            .map_err(|_| Error::Dash)?,
            16,
        ),
        1 => return Err(Error::Dash),
        value => (usize::try_from(value).map_err(|_| Error::Dash)?, 8),
    };
    if size < header || size > bytes.len() {
        return Err(Error::Dash);
    }
    let (item, rest) = bytes.split_at(size);
    *bytes = rest;
    Ok((&item[4..8], &item[header..]))
}

fn inspect_bmff_boxes(mut bytes: &[u8], depth: usize, budget: &mut usize) -> Result<()> {
    if depth > 32 {
        return Err(Error::Dash);
    }
    while !bytes.is_empty() {
        let (kind, payload) = next_bmff_box(&mut bytes, budget)?;
        inspect_bmff_box(kind, payload, depth, budget)?;
    }
    Ok(())
}

fn inspect_sample_descriptions(payload: &[u8], depth: usize, budget: &mut usize) -> Result<()> {
    // stsd is a full box followed by entry_count and actual SampleEntry boxes.
    let count = u32::from_be_bytes(
        payload
            .get(4..8)
            .ok_or(Error::Dash)?
            .try_into()
            .map_err(|_| Error::Dash)?,
    );
    if usize::try_from(count).map_err(|_| Error::Dash)? > *budget || depth > 32 {
        return Err(Error::Dash);
    }
    let mut entries = &payload[8..];
    for _ in 0..count {
        let (kind, entry) = next_bmff_box(&mut entries, budget)?;
        if matches!(kind, b"encv" | b"enca") {
            return Err(Error::DashDrmUnsupported);
        }
        if entry.len() < 8 {
            return Err(Error::Dash);
        }
        let child_offset = match kind {
            // VisualSampleEntry has 78 bytes of fields, including a free-text
            // compressor name. Only the following bytes contain child boxes.
            b"avc1" | b"avc2" | b"avc3" | b"avc4" | b"hvc1" | b"hev1" | b"av01" | b"vp08"
            | b"vp09" | b"mp4v" | b"dvav" | b"dva1" | b"dvhe" | b"dvh1" | b"vvc1" | b"vvi1" => 78,
            b"mp4a" | b"ac-3" | b"ec-3" | b"ac-4" | b"Opus" | b"fLaC" | b"alac" => {
                let version = u16::from_be_bytes(
                    entry
                        .get(8..10)
                        .ok_or(Error::Dash)?
                        .try_into()
                        .map_err(|_| Error::Dash)?,
                );
                match version {
                    0 => 28,
                    1 => 44,
                    2 => 64,
                    _ => return Err(Error::Dash),
                }
            }
            // Unknown sample entries and codec configuration boxes are opaque;
            // interpreting their binary/text contents as boxes invents markers.
            _ => continue,
        };
        inspect_bmff_boxes(
            entry.get(child_offset..).ok_or(Error::Dash)?,
            depth + 1,
            budget,
        )?;
    }
    if !entries.is_empty() {
        return Err(Error::Dash);
    }
    Ok(())
}

fn inspect_bmff_box(kind: &[u8], payload: &[u8], depth: usize, budget: &mut usize) -> Result<()> {
    match kind {
        b"pssh" | b"tenc" | b"senc" | b"encv" | b"enca" => Err(Error::DashDrmUnsupported),
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"mvex" | b"moof" | b"traf" | b"sinf"
        | b"schi" | b"edts" | b"dinf" | b"udta" | b"wave" => {
            inspect_bmff_boxes(payload, depth + 1, budget)
        }
        b"stsd" => inspect_sample_descriptions(payload, depth + 1, budget),
        _ => Ok(()),
    }
}

fn validate_fragment(mut bytes: &[u8], initialization: bool) -> Result<()> {
    let mut moov = false;
    let mut moof = false;
    let mut mdat = false;
    let mut budget = 4096;
    while !bytes.is_empty() {
        let (kind, payload) = next_bmff_box(&mut bytes, &mut budget)?;
        moov |= kind == b"moov";
        moof |= kind == b"moof";
        mdat |= kind == b"mdat" && !payload.is_empty();
        inspect_bmff_box(kind, payload, 0, &mut budget)?;
    }
    if if initialization {
        !moov || moof
    } else {
        !moof || !mdat
    } {
        return Err(Error::Dash);
    }
    Ok(())
}

async fn download_with_progress(
    request: &NewTask,
    id: TaskId,
    options: &DashOptions,
    cancel: CancellationToken,
    progress: DownloadProgress,
    tool_directory: Option<&Path>,
    work_directory: Option<&std::path::Path>,
) -> Result<DashOutcome> {
    let network = Network::new(request, options, cancel.clone(), progress.clone())?;
    let (manifest, bytes, url) = network.manifest().await?;
    let tracks = selected_tracks(&manifest, options)?;
    let installed = LocalTools::installed_directory();
    let tools = LocalTools::new(
        tool_directory
            .or(installed.as_deref())
            .ok_or(Error::MediaToolUnavailable)?,
    )
    .await?;
    fs::create_dir_all(&request.destination).await?;
    let directory = work_directory
        .map(std::path::Path::to_owned)
        .unwrap_or_else(|| request.destination.join(format!(".ndm-next-{id}")));
    fs::create_dir_all(&directory).await?;
    crate::cache::validate_directory(&directory)?;
    let metadata = fs::symlink_metadata(&directory).await?;
    if !metadata.is_dir() || reparse(&metadata) {
        return Err(Error::DashRecovery);
    }
    let mut plan = Sha256::new();
    plan.update(bytes);
    plan.update(url.as_str());
    plan.update(serde_json::to_vec(&(
        options.representation_index,
        &options.audio_indices,
    ))?);
    let plan = format!("{:x}", plan.finalize());
    let source_digest = digest(request.url.as_bytes());
    let checkpoint_path = directory.join("dash-checkpoint.json");
    let mut checkpoint = if regular_file(&checkpoint_path).await? {
        if fs::metadata(&checkpoint_path).await?.len() > MAX_CHECKPOINT_BYTES {
            return Err(Error::DashRecovery);
        }
        let mut value: Checkpoint = serde_json::from_slice(&fs::read(&checkpoint_path).await?)
            .map_err(|_| Error::DashRecovery)?;
        if !value.source_digest.is_empty() && value.source_digest != source_digest {
            restart_owned_files(&directory).await?;
            Checkpoint {
                plan,
                source_digest,
                resources: BTreeMap::new(),
            }
        } else if value.plan != plan {
            return Err(Error::DashRecovery);
        } else {
            value.source_digest = source_digest;
            value
        }
    } else {
        Checkpoint {
            plan,
            source_digest,
            resources: BTreeMap::new(),
        }
    };
    // Without strong validators no previous media is reused. Restart every
    // selected resource together so stale and current tracks cannot be joined.
    if checkpoint
        .resources
        .values()
        .any(|receipt| receipt.etag.is_none())
    {
        checkpoint.resources.clear();
    }
    let mut inputs = Vec::new();
    let mut resource_index = 0;
    let mut owned = vec![checkpoint_path, directory.join("dash-checkpoint.tmp")];
    for (track_index, track) in tracks.iter().enumerate() {
        let path = directory.join(format!("dash-track-{track_index}.mp4"));
        regular_file(&path).await?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .await?;
        owned.push(path.clone());
        for resource in &track.resources {
            if cancel.is_cancelled() {
                save_checkpoint(&directory, &checkpoint).await?;
                return Err(Error::Cancelled);
            }
            let part = directory.join(format!("dash-resource-{resource_index}.part"));
            owned.push(part.clone());
            let cached = checkpoint.resources.get(&resource_index);
            let existing = if let Some(receipt) = cached {
                if !regular_file(&part).await?
                    || receipt.length > MAX_RESOURCE_BYTES as u64
                    || fs::metadata(&part).await?.len() != receipt.length
                {
                    return Err(Error::DashRecovery);
                }
                let bytes = fs::read(&part).await?;
                if bytes.len() as u64 != receipt.length || digest(&bytes) != receipt.digest {
                    return Err(Error::DashRecovery);
                }
                Some(bytes)
            } else {
                None
            };
            let fetched = match network
                .fetch(
                    &resource.url,
                    resource.range,
                    MAX_RESOURCE_BYTES,
                    cached.and_then(|receipt| receipt.etag.as_deref()),
                )
                .await
            {
                Ok(fetched) => fetched,
                Err(error) => {
                    save_checkpoint(&directory, &checkpoint).await?;
                    return Err(error);
                }
            };
            let data = if fetched.unchanged {
                let bytes = existing.ok_or(Error::DashRecovery)?;
                progress.add_existing(bytes.len() as u64);
                bytes
            } else {
                if let Some(receipt) = cached {
                    // Some CDNs return 200 for conditional GET. Accept that
                    // response only after both its validator and its complete
                    // body match the previously verified resource.
                    if fetched.etag != receipt.etag
                        || fetched.bytes.len() as u64 != receipt.length
                        || digest(&fetched.bytes) != receipt.digest
                    {
                        return Err(Error::DashRecovery);
                    }
                } else {
                    validate_fragment(&fetched.bytes, resource.initialization)?;
                    write_file(&part, &fetched.bytes).await?;
                    checkpoint.resources.insert(
                        resource_index,
                        Receipt {
                            etag: fetched.etag,
                            digest: digest(&fetched.bytes),
                            length: fetched.bytes.len() as u64,
                        },
                    );
                    if resource_index % 8 == 0 {
                        save_checkpoint(&directory, &checkpoint).await?;
                    }
                }
                fetched.bytes
            };
            validate_fragment(&data, resource.initialization)?;
            file.write_all(&data).await?;
            resource_index += 1;
        }
        file.sync_all().await?;
        save_checkpoint(&directory, &checkpoint).await?;
        inputs.push(LocalInput {
            path,
            kind: Some(track.kind),
            language: track.language.clone(),
            title: track.label.clone(),
            default: track.default,
        });
    }
    progress.set_total(Some(progress.snapshot().downloaded));
    let prefix = directory.join("dash-output");
    for extension in ["mp4", "mkv"] {
        let path = prefix.with_extension(extension);
        if regular_file(&path).await? {
            fs::remove_file(&path).await?;
        }
        owned.push(path);
    }
    let (output, tracks) = remux_local(
        &tools,
        &inputs,
        &prefix,
        manifest.catalog.duration_ms,
        &cancel,
        &progress,
    )
    .await?;
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let suggested = safe_filename(request.suggested_filename.as_deref().unwrap_or("video.mp4"));
    let stem = Path::new(&suggested)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("video");
    let extension = output
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or(Error::Dash)?;
    let path = publish_download(
        &output,
        &request.destination,
        &format!("{stem}.{extension}"),
        &progress,
        &cancel,
    )
    .await?;
    for file in owned {
        let _ = fs::remove_file(file).await;
    }
    let _ = fs::remove_dir(directory).await;
    Ok(DashOutcome { path, tracks })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn sample_entry_init(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut entries = vec![0; 4]; // stsd full-box version/flags
        entries.extend_from_slice(&1_u32.to_be_bytes());
        entries.extend(boxed(kind, payload));
        let mut bytes = boxed(b"stsd", &entries);
        for kind in [b"stbl", b"minf", b"mdia", b"trak", b"moov"] {
            bytes = boxed(kind, &bytes);
        }
        bytes
    }

    fn fixture_initialization_with_movie_child(child: &[u8]) -> Vec<u8> {
        let fixture = include_bytes!("../tests/fixtures/dash/init-stream0.m4s");
        let moov_offset = u32::from_be_bytes(fixture[..4].try_into().unwrap()) as usize;
        assert_eq!(&fixture[moov_offset + 4..moov_offset + 8], b"moov");
        let moov_size =
            u32::from_be_bytes(fixture[moov_offset..moov_offset + 4].try_into().unwrap()) as usize;
        let mut initialization = fixture.to_vec();
        initialization.splice(
            moov_offset + moov_size..moov_offset + moov_size,
            child.iter().copied(),
        );
        initialization[moov_offset..moov_offset + 4]
            .copy_from_slice(&((moov_size + child.len()) as u32).to_be_bytes());
        initialization
    }

    #[test]
    fn clear_initialization_metadata_does_not_misidentify_drm_markers() {
        let mut text = b"essence latency encv enca pssh tenc senc".to_vec();
        // Even a box-shaped byte sequence remains opaque inside a metadata leaf.
        text.extend(boxed(b"pssh", &[0; 24]));
        let metadata = boxed(b"name", &text);
        let initialization = fixture_initialization_with_movie_child(&boxed(b"udta", &metadata));
        assert!(validate_fragment(&initialization, true).is_ok());

        let mut video = vec![0; 78]; // VisualSampleEntry fields
        video[42..49].copy_from_slice(b"essence"); // compressor name text
        video.extend(boxed(b"avcC", b"latency essence")); // opaque codec config
        assert!(validate_fragment(&sample_entry_init(b"avc1", &video), true).is_ok());
    }

    #[test]
    fn actual_encrypted_boxes_and_sample_entry_types_are_rejected() {
        for (kind, size) in [(b"encv", 78), (b"enca", 28)] {
            let initialization = sample_entry_init(kind, &vec![0; size]);
            assert!(matches!(
                validate_fragment(&initialization, true),
                Err(Error::DashDrmUnsupported)
            ));
        }
        let initialization = fixture_initialization_with_movie_child(&boxed(b"pssh", &[0; 24]));
        assert!(matches!(
            validate_fragment(&initialization, true),
            Err(Error::DashDrmUnsupported)
        ));
        let encrypted = boxed(b"moof", &boxed(b"traf", &boxed(b"senc", &[0; 8])));
        let mut fragment = encrypted;
        fragment.extend(boxed(b"mdat", b"encrypted payload"));
        assert!(matches!(
            validate_fragment(&fragment, false),
            Err(Error::DashDrmUnsupported)
        ));

        let mut video = vec![0; 78];
        video.extend(boxed(b"sinf", &boxed(b"schi", &boxed(b"tenc", &[0; 24]))));
        assert!(matches!(
            validate_fragment(&sample_entry_init(b"avc1", &video), true),
            Err(Error::DashDrmUnsupported)
        ));
    }

    #[test]
    fn malformed_box_lengths_and_sample_entry_counts_are_rejected() {
        let mut malformed = boxed(b"moov", &boxed(b"trak", &[]));
        malformed[8..12].copy_from_slice(&1000_u32.to_be_bytes());
        assert!(matches!(
            validate_fragment(&malformed, true),
            Err(Error::Dash)
        ));
        let mut entries = vec![0; 4];
        entries.extend_from_slice(&2_u32.to_be_bytes());
        entries.extend(boxed(b"avc1", &[0; 78]));
        let malformed = boxed(
            b"moov",
            &boxed(
                b"trak",
                &boxed(
                    b"mdia",
                    &boxed(b"minf", &boxed(b"stbl", &boxed(b"stsd", &entries))),
                ),
            ),
        );
        assert!(matches!(
            validate_fragment(&malformed, true),
            Err(Error::Dash)
        ));
    }

    #[test]
    fn audio_sample_entry_children_follow_version_specific_fields() {
        for (version, size) in [(0_u16, 28), (1, 44), (2, 64)] {
            let mut audio = vec![0; size];
            audio[8..10].copy_from_slice(&version.to_be_bytes());
            audio.extend(boxed(b"esds", b"essence latency"));
            assert!(validate_fragment(&sample_entry_init(b"mp4a", &audio), true).is_ok());
            audio.extend(boxed(b"sinf", &boxed(b"schi", &boxed(b"tenc", &[0; 24]))));
            assert!(matches!(
                validate_fragment(&sample_entry_init(b"mp4a", &audio), true),
                Err(Error::DashDrmUnsupported)
            ));
        }
    }

    #[test]
    fn box_inspection_bounds_sizes_depth_and_total_box_count() {
        let child = boxed(b"free", b"latency essence");
        let mut extended = 1_u32.to_be_bytes().to_vec();
        extended.extend_from_slice(b"moov");
        extended.extend_from_slice(&((16 + child.len()) as u64).to_be_bytes());
        extended.extend(child);
        assert!(validate_fragment(&extended, true).is_ok());
        extended[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(matches!(
            validate_fragment(&extended, true),
            Err(Error::Dash)
        ));

        let mut nested = boxed(b"free", &[]);
        for _ in 0..34 {
            nested = boxed(b"moov", &nested);
        }
        assert!(matches!(validate_fragment(&nested, true), Err(Error::Dash)));
        let children = boxed(b"free", &[]).repeat(4096);
        assert!(matches!(
            validate_fragment(&boxed(b"moov", &children), true),
            Err(Error::Dash)
        ));
    }
}
