//! Shared local-only media inspection and remuxing. No remote address or header is
//! passed to a media tool; installed tools are built without network protocols.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use serde::Deserialize;
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
};
use tokio_util::sync::CancellationToken;

use crate::{DownloadProgress, Error, Result};

pub(crate) struct LocalTools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl LocalTools {
    pub(crate) async fn new(directory: &Path) -> Result<Self> {
        let name = |stem: &str| {
            if cfg!(windows) {
                format!("{stem}.exe")
            } else {
                stem.to_owned()
            }
        };
        Ok(Self {
            ffmpeg: fs::canonicalize(directory.join(name("ffmpeg")))
                .await
                .map_err(|_| Error::MediaToolUnavailable)?,
            ffprobe: fs::canonicalize(directory.join(name("ffprobe")))
                .await
                .map_err(|_| Error::MediaToolUnavailable)?,
        })
    }

    pub(crate) fn installed_directory() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()?
            .parent()
            .map(|directory| directory.join("ffmpeg"))
    }
}

pub(crate) struct LocalInput {
    pub path: PathBuf,
    /// None keeps all audio and video streams from this input.
    pub kind: Option<&'static str>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
}

fn command(tool: &Path) -> Command {
    let mut command = Command::new(tool);
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
    format: Option<Format>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
}

struct Metadata {
    tracks: Vec<(String, String)>,
    duration_ms: Option<u64>,
}

async fn probe(tools: &LocalTools, path: &Path, cancel: &CancellationToken) -> Result<Metadata> {
    let mut command = command(&tools.ffprobe);
    command
        .args([
            "-v",
            "error",
            "-max_alloc",
            "67108864",
            "-max_streams",
            "256",
            "-protocol_whitelist",
            "file",
            "-show_entries",
            "stream=codec_type,codec_name:format=duration",
            "-of",
            "json",
        ])
        .arg(path);
    command.stdout(Stdio::piped());
    let mut child = command.spawn().map_err(|_| Error::Dash)?;
    let stdout = child.stdout.take().ok_or(Error::Dash)?;
    let mut stdout = stdout.take(64 * 1024 + 1);
    let mut bytes = Vec::new();
    tokio::select! {
        biased;
        _ = cancel.cancelled() => { let _ = child.kill().await; return Err(Error::Cancelled); }
        result = stdout.read_to_end(&mut bytes) => { result.map_err(|_| Error::Dash)?; }
    };
    if bytes.len() > 64 * 1024 {
        let _ = child.kill().await;
        return Err(Error::Dash);
    }
    let status = tokio::select! {
        biased;
        _ = cancel.cancelled() => { let _ = child.kill().await; return Err(Error::Cancelled); }
        status = child.wait() => status.map_err(|_| Error::Dash)?,
    };
    if !status.success() {
        return Err(Error::Dash);
    }
    let parsed: Probe = serde_json::from_slice(&bytes).map_err(|_| Error::Dash)?;
    if parsed.streams.is_empty() || parsed.streams.len() > 256 {
        return Err(Error::Dash);
    }
    let duration_ms = parsed
        .format
        .and_then(|format| format.duration)
        .and_then(|duration| duration.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0 && *duration <= 604_800.0)
        .map(|duration| (duration * 1000.0).round() as u64);
    Ok(Metadata {
        tracks: parsed
            .streams
            .into_iter()
            .map(|stream| {
                (
                    stream.codec_type.unwrap_or_default(),
                    stream.codec_name.unwrap_or_default(),
                )
            })
            .collect(),
        duration_ms,
    })
}

fn reported_timestamp(line: &str) -> Option<u64> {
    // FFmpeg reports microseconds, despite the historical out_time_ms spelling.
    line.trim()
        .strip_prefix("out_time_us=")?
        .parse::<u64>()
        .ok()
        .map(|value| value / 1000)
}

async fn remux_process(
    command: &mut Command,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
) -> Result<bool> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    command.stdout(Stdio::piped());
    let mut child = command.spawn().map_err(|_| Error::Dash)?;
    let stdout = child.stdout.take().ok_or(Error::Dash)?;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = tokio::select! {
            biased;
            _ = cancel.cancelled() => { let _ = child.kill().await; return Err(Error::Cancelled); }
            line = lines.next_line() => line.map_err(|_| Error::Dash)?,
        };
        let Some(line) = line else { break };
        if line.len() > 1024 {
            return Err(Error::Dash);
        }
        if let Some(timestamp) = reported_timestamp(&line) {
            progress.set_finalized_ms(timestamp);
        }
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => { let _ = child.kill().await; Err(Error::Cancelled) },
        status = child.wait() => status.map(|status| status.success()).map_err(|_| Error::Dash),
    }
}

fn language_tag(language: &str) -> &'static str {
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

/// Return a validated complete local output. The caller owns staging cleanup and
/// publishes the file atomically only after this succeeds.
pub(crate) async fn remux_local(
    tools: &LocalTools,
    inputs: &[LocalInput],
    output_prefix: &Path,
    duration_hint_ms: Option<u64>,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
) -> Result<(PathBuf, usize)> {
    let mut expected = Vec::new();
    let mut local_duration = None;
    for input in inputs {
        let metadata = probe(tools, &input.path, cancel).await?;
        local_duration = local_duration.into_iter().chain(metadata.duration_ms).max();
        let tracks: Vec<_> = metadata
            .tracks
            .into_iter()
            .filter(|(kind, _)| {
                input
                    .kind
                    .map_or(matches!(kind.as_str(), "audio" | "video"), |wanted| {
                        kind == wanted
                    })
            })
            .collect();
        if tracks.is_empty()
            || tracks
                .iter()
                .any(|(_, codec)| codec.is_empty() || codec == "unknown")
        {
            return Err(Error::Dash);
        }
        expected.extend(tracks);
    }
    if expected.is_empty() || expected.len() > 256 {
        return Err(Error::Dash);
    }
    expected.sort_unstable();
    // The denominator comes from the staged local media whenever it is available.
    let duration = local_duration.or(duration_hint_ms);
    for extension in ["mp4", "mkv"] {
        let output = output_prefix.with_extension(extension);
        let mut command = command(&tools.ffmpeg);
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-xerror",
            "-nostdin",
            "-nostats",
            "-stats_period",
            "0.1",
            "-progress",
            "pipe:1",
            "-max_alloc",
            "67108864",
        ]);
        for input in inputs {
            command
                .args(["-protocol_whitelist", "file", "-max_streams", "256", "-i"])
                .arg(&input.path);
        }
        let mut audio_index = 0;
        for (index, input) in inputs.iter().enumerate() {
            match input.kind {
                Some("video") => {
                    command.args(["-map", &format!("{index}:v")]);
                }
                Some("audio") => {
                    command.args(["-map", &format!("{index}:a")]);
                }
                None => {
                    command.args([
                        "-map",
                        &format!("{index}:v?"),
                        "-map",
                        &format!("{index}:a?"),
                    ]);
                }
                _ => return Err(Error::Dash),
            }
            if input.kind == Some("audio") {
                if let Some(language) = &input.language {
                    command.args([
                        &format!("-metadata:s:a:{audio_index}"),
                        &format!("language={}", language_tag(language)),
                    ]);
                }
                if let Some(title) = &input.title {
                    command.args([
                        &format!("-metadata:s:a:{audio_index}"),
                        &format!("title={title}"),
                    ]);
                }
                command
                    .arg(format!("-disposition:a:{audio_index}"))
                    .arg(if input.default { "default" } else { "0" });
                audio_index += 1;
            }
        }
        command.args(["-c", "copy", "-n"]).arg(&output);
        progress.begin_finalization(duration);
        if !remux_process(&mut command, cancel, progress).await? {
            continue;
        }
        let metadata = match probe(tools, &output, cancel).await {
            Ok(metadata) => metadata,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => continue,
        };
        let mut actual = metadata.tracks;
        actual.sort_unstable();
        if actual != expected {
            continue;
        }
        if let (Some(before), Some(after)) = (local_duration, metadata.duration_ms)
            && after.saturating_add((before / 20).max(250)) < before
        {
            continue;
        }
        let mut file = fs::File::open(&output).await?;
        let mut header = [0_u8; 8];
        let valid = file.read_exact(&mut header).await.is_ok()
            && if extension == "mp4" {
                &header[4..8] == b"ftyp"
            } else {
                header[..4] == [0x1a, 0x45, 0xdf, 0xa3]
            };
        if valid {
            return Ok((output, actual.len()));
        }
    }
    Err(Error::Dash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_media_tool_timestamps_advance_finalization() {
        assert_eq!(reported_timestamp("out_time_us=1234567"), Some(1234));
        for line in [
            "progress=end",
            "out_time_us=N/A",
            "out_time_us=-1",
            "out_time_ms=1234000",
        ] {
            assert_eq!(reported_timestamp(line), None);
        }
    }
}
