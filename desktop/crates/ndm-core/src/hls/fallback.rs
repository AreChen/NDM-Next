//! Remux finite, validated HLS media using local files.

use std::path::Path;

use hls_engine::crypto::resource::{
    ClearContainer, ResourceOptions, ResourceRequest, ResourceSession,
};
use tokio::io::AsyncWriteExt;

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

pub(super) struct Selection<'a> {
    pub inputs: &'a [(InputId, SourceLocation)],
    pub audio: &'a [m3u8_rs::AlternativeMedia],
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
    use crate::media::{LocalInput, LocalTools, remux_local};

    let tools = LocalTools::new(tool_dir).await?;
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
        let audio = index
            .checked_sub(1)
            .and_then(|index| selected.audio.get(index));
        inputs.push(LocalInput {
            path,
            kind: if selected.inputs.len() == 1 {
                None
            } else if index == 0 {
                Some("video")
            } else {
                Some("audio")
            },
            language: audio.and_then(|audio| audio.language.clone()),
            title: audio.map(|audio| audio.name.clone()),
            default: audio.is_some_and(|audio| audio.default),
        });
    }
    source.progress.finalize();
    staged.file("output.mp4");
    staged.file("output.mkv");
    let prefix = staged.root.join("output");
    let remux_cancel = source.cancel.child_token();
    let remux = remux_local(
        &tools,
        &inputs,
        &prefix,
        None,
        &remux_cancel,
        &source.progress,
    );
    tokio::pin!(remux);
    let result = tokio::select! {
        biased;
        _ = stop.cancelled() => { remux_cancel.cancel(); remux.await }
        result = &mut remux => result,
    };
    let (output, tracks) = result.map_err(|error| {
        if matches!(error, Error::Cancelled | Error::MediaToolUnavailable) {
            error
        } else {
            Error::Hls
        }
    })?;
    stopped(&source, &stop)?;
    let suggested = safe_filename(request.suggested_filename.as_deref().unwrap_or("video.mp4"));
    let stem = Path::new(&suggested)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("video");
    let extension = output
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or(Error::Hls)?;
    let target = publish_download(
        &output,
        &request.destination,
        &format!("{stem}.{extension}"),
        &source.progress,
        &source.cancel,
    )
    .await?;
    super::recovery::remove_files(work_dir).await;
    drop(staged);
    let _ = fs::remove_dir(work_dir).await;
    Ok(HlsOutcome {
        path: target,
        tracks,
        stopped: false,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::Path as WebPath, routing::get};

    #[tokio::test]
    #[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
    async fn local_remux_reports_observed_time_against_local_duration() {
        let tools =
            std::env::var_os("NDM_TEST_FFMPEG_DIR").expect("NDM_TEST_FFMPEG_DIR is required");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/{*path}",
                    get(|WebPath(path): WebPath<String>| async move {
                        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls");
                        tokio::fs::read(root.join(path)).await.unwrap()
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let work = directory.path().join("work");
        fs::create_dir(&work).await.unwrap();
        let progress = DownloadProgress::default();
        let request = NewTask {
            url: format!("http://{address}/mpeg2/index.m3u8"),
            destination: directory.path().to_owned(),
            suggested_filename: Some("video.mp4".into()),
            source_url: None,
            expected_size: None,
            headers: Default::default(),
        };
        let (source, url) = network_source(
            &request,
            CancellationToken::new(),
            None,
            None,
            progress.clone(),
        )
        .unwrap();
        let inputs = vec![(InputId::new("primary").unwrap(), SourceLocation::Url(url))];
        remux(
            &request,
            TaskId::new_v4(),
            source,
            Selection {
                inputs: &inputs,
                audio: &[],
            },
            &work,
            Path::new(&tools),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let snapshot = progress.snapshot();
        assert!(
            snapshot.finalized_ms >= 2000,
            "must report the tool's observed media time"
        );
        assert!(
            snapshot
                .finalization_total_ms
                .is_some_and(|duration| (2000..=4000).contains(&duration))
        );
    }
}
