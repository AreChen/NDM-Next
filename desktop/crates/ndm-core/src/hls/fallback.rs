//! Remux finite, validated HLS media using local, network-disabled media tools.

use std::{path::Path, process::Stdio};

use hls_engine::crypto::resource::{
    ClearContainer, ResourceOptions, ResourceRequest, ResourceSession,
};
use tokio::{io::AsyncWriteExt, process::Command};

use super::*;

/// Remove only names created by this operation. Never recursively remove user content.
struct Staging {
    root: PathBuf,
    files: Vec<PathBuf>,
}

impl Staging {
    async fn new(work_dir: &Path) -> Result<Self> {
        let root = work_dir.join(format!("media-{}", TaskId::new_v4()));
        fs::create_dir(&root).await?;
        Ok(Self {
            root: fs::canonicalize(root).await?,
            files: Vec::new(),
        })
    }

    fn file(&mut self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        self.files.push(path.clone());
        path
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        for file in &self.files {
            let _ = std::fs::remove_file(file);
        }
        let _ = std::fs::remove_dir(&self.root);
    }
}

fn stopped(source: &NetworkSource, stop: &CancellationToken) -> Result<()> {
    if source.cancel.is_cancelled() || stop.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

async fn stage_track(
    source: Arc<NetworkSource>,
    session: &ResourceSession,
    input: &(InputId, SourceLocation),
    path: &Path,
    stop: &CancellationToken,
) -> Result<()> {
    stopped(&source, stop)?;
    let text = source.read_text(&input.1).await.map_err(|_| {
        if source.cancel.is_cancelled() {
            Error::Cancelled
        } else {
            Error::Hls
        }
    })?;
    let snapshot = parse_playlist_snapshot(&text, PlaylistContext::new(input.0.clone(), 0))
        .map_err(|_| Error::Hls)?;
    snapshot.validate_finite_vod().map_err(|_| Error::Hls)?;
    if snapshot.segments().len() > 4096 {
        return Err(Error::Hls);
    }
    let first = snapshot.segments().first().ok_or(Error::Hls)?;
    let initial_map = first.map();
    if snapshot
        .segments()
        .iter()
        .any(|item| item.map() != initial_map)
    {
        return Err(Error::Hls);
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await?;
    if initial_map.is_some() {
        let map = ResourceRequest::map(&snapshot, 0).map_err(|_| Error::Hls)?;
        let data = session
            .read(source.clone(), map)
            .await
            .map_err(|_| Error::Hls)?;
        if data.container() != ClearContainer::Fmp4Init {
            return Err(Error::Hls);
        }
        file.write_all(data.bytes()).await?;
    }
    let mut expected = None;
    for index in 0..snapshot.segments().len() {
        stopped(&source, stop)?;
        let resource = ResourceRequest::media(&snapshot, index).map_err(|_| Error::Hls)?;
        let data = session.read(source.clone(), resource).await.map_err(|_| {
            if source.cancel.is_cancelled() {
                Error::Cancelled
            } else {
                Error::Hls
            }
        })?;
        let container = data.container();
        match container {
            ClearContainer::TransportStream | ClearContainer::PackedAac
                if initial_map.is_none() => {}
            ClearContainer::Fmp4Media if initial_map.is_some() => {}
            _ => return Err(Error::Hls),
        }
        if expected.is_some_and(|old| old != container) {
            return Err(Error::Hls);
        }
        expected = Some(container);
        file.write_all(data.bytes()).await?;
    }
    file.flush().await?;
    Ok(())
}

fn command(tool: &Path) -> Command {
    let mut cmd = Command::new(tool);
    cmd.stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

async fn run(
    cmd: &mut Command,
    source: &NetworkSource,
    stop: &CancellationToken,
) -> Result<std::process::Output> {
    stopped(source, stop)?;
    tokio::select! {
        _ = source.cancel.cancelled() => Err(Error::Cancelled),
        _ = stop.cancelled() => Err(Error::Cancelled),
        result = cmd.output() => result.map_err(|_| Error::Hls),
    }
}

#[derive(Deserialize)]
struct ProbeResult {
    streams: Vec<ProbeStream>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_name: Option<String>,
    codec_type: Option<String>,
}

pub(super) struct Selection<'a> {
    pub inputs: &'a [(InputId, SourceLocation)],
    pub audio: &'a [m3u8_rs::AlternativeMedia],
}

fn media_language(language: &str) -> &'static str {
    let primary = language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    isolang::Language::from_639_1(&primary)
        .or_else(|| isolang::Language::from_639_3(&primary))
        .map(|language| language.to_639_3())
        .unwrap_or("und")
}

async fn probe(
    tool: &Path,
    output: &Path,
    source: &NetworkSource,
    stop: &CancellationToken,
) -> Result<Vec<(String, String)>> {
    let mut cmd = command(tool);
    cmd.args([
        "-v",
        "error",
        "-max_streams",
        "256",
        "-show_entries",
        "stream=codec_type,codec_name",
        "-of",
        "json",
    ])
    .arg(output);
    let result = run(&mut cmd, source, stop).await?;
    if !result.status.success() || result.stdout.len() > 16 * 1024 {
        return Err(Error::Hls);
    }
    let parsed: ProbeResult = serde_json::from_slice(&result.stdout).map_err(|_| Error::Hls)?;
    if parsed.streams.is_empty() || parsed.streams.len() > 256 {
        return Err(Error::Hls);
    }
    Ok(parsed
        .streams
        .into_iter()
        .map(|stream| {
            (
                stream.codec_type.unwrap_or_default(),
                stream.codec_name.unwrap_or_default(),
            )
        })
        .collect())
}

pub(super) async fn remux(
    request: &NewTask,
    id: TaskId,
    source: Arc<NetworkSource>,
    selected: Selection<'_>,
    work_dir: &Path,
    tool_dir: &Path,
    stop: CancellationToken,
) -> Result<HlsOutcome> {
    let executable = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let probe_name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let ffmpeg = fs::canonicalize(tool_dir.join(executable))
        .await
        .map_err(|_| Error::Hls)?;
    let ffprobe = fs::canonicalize(tool_dir.join(probe_name))
        .await
        .map_err(|_| Error::Hls)?;
    let keys = KeySession::new(
        id.to_string(),
        "hls-fallback",
        Arc::new(Keys(source.clone())),
        Arc::new(Clock(Instant::now())),
        KeySessionOptions::default(),
    )
    .map_err(|_| Error::Hls)?;
    let session = ResourceSession::new(
        keys,
        ResourceOptions::default().with_limits(
            MAX_RESOURCE_BYTES as u64,
            MAX_RESOURCE_BYTES as u64,
            1,
        ),
    )
    .map_err(|_| Error::Hls)?;
    let mut staged = Staging::new(work_dir).await?;
    let mut inputs = Vec::new();
    for (index, selected_input) in selected.inputs.iter().enumerate() {
        let path = staged.file(&format!("track-{index}.bin"));
        stage_track(source.clone(), &session, selected_input, &path, &stop).await?;
        inputs.push(path);
    }
    let mut expected = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        for (kind, codec) in probe(&ffprobe, input, &source, &stop).await? {
            let keep = if inputs.len() == 1 {
                matches!(kind.as_str(), "video" | "audio")
            } else if index == 0 {
                kind == "video"
            } else {
                kind == "audio"
            };
            if keep {
                if codec.is_empty() || codec == "unknown" {
                    return Err(Error::Hls);
                }
                expected.push((kind, codec));
            }
        }
    }
    if expected.is_empty() || expected.len() > 256 {
        return Err(Error::Hls);
    }
    expected.sort_unstable();

    for extension in ["mp4", "mkv"] {
        let output = staged.file(&format!("output.{extension}"));
        let mut cmd = command(&ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-xerror", "-nostdin"]);
        for input in &inputs {
            cmd.args(["-max_streams", "256", "-i"]).arg(input);
        }
        if inputs.len() == 1 {
            cmd.args(["-map", "0:v?", "-map", "0:a?"]);
        } else {
            cmd.args(["-map", "0:v?"]);
            for index in 1..inputs.len() {
                cmd.args(["-map", &format!("{index}:a")]);
            }
            for (index, audio) in selected.audio.iter().enumerate() {
                if let Some(language) = &audio.language {
                    cmd.args([
                        &format!("-metadata:s:a:{index}"),
                        &format!("language={}", media_language(language)),
                    ]);
                }
                cmd.args([
                    &format!("-metadata:s:a:{index}"),
                    &format!("title={}", audio.name),
                ]);
                cmd.arg(format!("-disposition:a:{index}"))
                    .arg(if audio.default { "default" } else { "0" });
            }
        }
        cmd.args(["-c", "copy", "-n"]).arg(&output);
        if !run(&mut cmd, &source, &stop).await?.status.success() {
            continue;
        }
        let mut actual = match probe(&ffprobe, &output, &source, &stop).await {
            Ok(tracks) => tracks,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => continue,
        };
        actual.sort_unstable();
        if actual != expected {
            continue;
        }
        let tracks = actual.len();
        let mut file = fs::File::open(&output).await?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).await?;
        let valid = if extension == "mp4" {
            &header[4..8] == b"ftyp"
        } else {
            header[..4] == [0x1a, 0x45, 0xdf, 0xa3]
        };
        if !valid {
            continue;
        }
        drop(file);
        let suggested = safe_filename(request.suggested_filename.as_deref().unwrap_or("video.mp4"));
        let stem = Path::new(&suggested)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("video");
        let target = publish_no_overwrite(
            &output,
            &request.destination,
            &format!("{stem}.{extension}"),
        )
        .await?;
        super::recovery::remove_files(work_dir).await;
        drop(staged);
        let _ = fs::remove_dir(work_dir).await;
        return Ok(HlsOutcome {
            path: target,
            tracks,
            stopped: false,
        });
    }
    Err(Error::Hls)
}
