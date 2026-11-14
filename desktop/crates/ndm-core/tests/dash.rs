use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{Path as WebPath, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use ndm_core::{
    DashOptions, Error, NewTask, Scheduler, SchedulerOptions, Store, TaskId, TaskState,
    download_dash_with_media_tool, inspect_dash,
};
use tokio_util::sync::CancellationToken;

fn request(dir: &Path, url: String) -> NewTask {
    NewTask {
        url,
        destination: dir.to_owned(),
        suggested_filename: Some("video.mp4".into()),
        source_url: None,
        expected_size: None,
        headers: BTreeMap::new(),
    }
}

#[tokio::test]
async fn auto_mpd_does_not_publish_the_manifest_as_a_completed_video() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/video.mpd",
                get(|| async { "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"dynamic\"/>" }),
            ),
        )
        .await
        .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let id = TaskId::new_v4();
    store
        .prepare(
            id,
            &request(dir.path(), format!("http://{address}/video.mpd")),
        )
        .unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    let task = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let task = store.get(id).unwrap().unwrap();
            if matches!(task.state, TaskState::Completed | TaskState::Failed) {
                break task;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    scheduler.shutdown().await;
    assert_eq!(task.state, TaskState::Failed);
    assert!(!dir.path().join("video.mp4").exists());
}

struct Fixture {
    root: PathBuf,
    manifest: String,
    missing: bool,
    truncated: bool,
    emit_etag: bool,
    ignore_conditionals: bool,
    slow: AtomicBool,
    version: AtomicUsize,
    segment_requests: AtomicUsize,
    cache_hits: AtomicUsize,
    headers: Mutex<Vec<HeaderMap>>,
}

impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dash");
        let manifest = std::fs::read_to_string(root.join("manifest.mpd"))
            .unwrap()
            .replace(
                "<Period id=\"0\" start=\"PT0.0S\">",
                "<Period id=\"0\" start=\"PT0.0S\"><BaseURL>../media/</BaseURL>",
            );
        Self {
            root,
            manifest,
            missing: false,
            truncated: false,
            emit_etag: true,
            ignore_conditionals: false,
            slow: AtomicBool::new(false),
            version: AtomicUsize::new(1),
            segment_requests: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
            headers: Mutex::new(Vec::new()),
        }
    }
}

async fn serve(
    State(fixture): State<Arc<Fixture>>,
    WebPath(path): WebPath<String>,
    headers: HeaderMap,
) -> Response {
    fixture.headers.lock().unwrap().push(headers.clone());
    if path == "entry" {
        return Response::builder()
            .status(StatusCode::FOUND)
            .header("Location", "/manifests/manifest.mpd")
            .body(Body::empty())
            .unwrap();
    }
    if path.starts_with("manifests/") && path.ends_with(".mpd") {
        return Response::builder()
            .header("Content-Type", "application/dash+xml")
            .body(Body::from(fixture.manifest.clone()))
            .unwrap();
    }
    let Some(name) = path
        .strip_prefix("media/")
        .filter(|name| !name.contains('/') && !name.contains('\\'))
    else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    };
    if name.starts_with("chunk") {
        fixture.segment_requests.fetch_add(1, Ordering::Relaxed);
        if fixture.slow.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if fixture.missing {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap();
        }
    }
    let etag = format!("\"fixture-{}\"", fixture.version.load(Ordering::Relaxed));
    if fixture.emit_etag
        && !fixture.ignore_conditionals
        && headers
            .get("If-None-Match")
            .is_some_and(|value| value == etag.as_str())
    {
        fixture.cache_hits.fetch_add(1, Ordering::Relaxed);
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .unwrap();
    }
    let Ok(mut bytes) = tokio::fs::read(fixture.root.join(name)).await else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    };
    if fixture.truncated && name.starts_with("chunk") {
        bytes.truncate(bytes.len().saturating_sub(10));
    }
    let mut response = Response::builder();
    if fixture.emit_etag {
        response = response.header("ETag", etag);
    }
    response.body(Body::from(bytes)).unwrap()
}

async fn server(fixture: Arc<Fixture>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/{*path}", get(serve))
                .with_state(fixture),
        )
        .await
        .unwrap();
    });
    format!("http://{address}")
}

fn tools() -> PathBuf {
    std::env::var_os("NDM_TEST_FFMPEG_DIR")
        .map(PathBuf::from)
        .expect("NDM_TEST_FFMPEG_DIR is required")
}

#[tokio::test]
async fn catalog_uses_final_manifest_address_and_lists_quality_and_language_choices() {
    let base = server(Arc::new(Fixture::new())).await;
    let directory = tempfile::tempdir().unwrap();
    let catalog = inspect_dash(
        &request(directory.path(), format!("{base}/entry")),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(catalog.representations.len(), 2);
    assert_eq!(catalog.representations[0].resolution, Some((160, 90)));
    assert_eq!(catalog.representations[1].resolution, Some((80, 44)));
    assert_eq!(
        catalog
            .audio_tracks
            .iter()
            .map(|track| track.language.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("eng"), Some("jpn")]
    );
    assert!(catalog.audio_tracks[0].default);
    assert_eq!(catalog.duration_ms, Some(3000));
}

#[tokio::test]
async fn protected_and_live_media_are_reported_explicitly_without_downloading_media() {
    for (manifest, protected) in [
        ("<MPD type=\"dynamic\"/>", false),
        ("<MPD><ContentProtection/></MPD>", true),
    ] {
        let mut fixture = Fixture::new();
        fixture.manifest = manifest.into();
        let fixture = Arc::new(fixture);
        let base = server(fixture.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let result = inspect_dash(
            &request(directory.path(), format!("{base}/manifests/manifest.mpd")),
            CancellationToken::new(),
        )
        .await;
        assert!(if protected {
            matches!(result, Err(Error::DashDrmUnsupported))
        } else {
            matches!(result, Err(Error::DashDynamicUnsupported))
        });
        assert_eq!(fixture.segment_requests.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn generated_dash_is_remuxed_with_selected_quality_and_multiple_audio_tracks() {
    let base = server(Arc::new(Fixture::new())).await;
    let directory = tempfile::tempdir().unwrap();
    let id = TaskId::new_v4();
    let result = download_dash_with_media_tool(
        &request(directory.path(), format!("{base}/entry")),
        id,
        &DashOptions {
            representation_index: Some(1),
            audio_indices: vec![0, 1],
            ..Default::default()
        },
        CancellationToken::new(),
        &tools(),
    )
    .await
    .unwrap();
    assert_eq!(result.tracks, 3);
    let metadata = std::process::Command::new(tools().join(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    }))
    .args([
        "-v",
        "error",
        "-count_packets",
        "-show_entries",
        "stream=codec_type,width,nb_read_packets:stream_tags=language:format=duration",
        "-of",
        "json",
    ])
    .arg(&result.path)
    .output()
    .unwrap();
    assert!(metadata.status.success());
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
    assert_eq!(metadata["streams"][0]["width"], 80);
    assert_eq!(metadata["streams"][0]["nb_read_packets"], "30");
    assert_eq!(metadata["streams"][1]["tags"]["language"], "eng");
    assert_eq!(metadata["streams"][2]["tags"]["language"], "jpn");
    assert!(
        metadata["format"]["duration"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap()
            >= 3.0
    );
    assert!(!directory.path().join(format!(".ndm-next-{id}")).exists());
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn missing_or_truncated_fragments_never_publish_a_completed_video() {
    for truncated in [false, true] {
        let mut fixture = Fixture::new();
        fixture.missing = !truncated;
        fixture.truncated = truncated;
        let base = server(Arc::new(fixture)).await;
        let directory = tempfile::tempdir().unwrap();
        let result = download_dash_with_media_tool(
            &request(directory.path(), format!("{base}/entry")),
            TaskId::new_v4(),
            &DashOptions::default(),
            CancellationToken::new(),
            &tools(),
        )
        .await;
        assert!(result.is_err());
        assert!(!directory.path().join("video.mp4").exists());
        assert!(!directory.path().join("video.mkv").exists());
    }
}

async fn paused_download(fixture: Arc<Fixture>, task: NewTask, id: TaskId) {
    fixture.slow.store(true, Ordering::Relaxed);
    let cancellation = CancellationToken::new();
    let worker_cancel = cancellation.clone();
    let worker = tokio::spawn(async move {
        download_dash_with_media_tool(&task, id, &DashOptions::default(), worker_cancel, &tools())
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.segment_requests.load(Ordering::Relaxed) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    assert!(matches!(worker.await.unwrap(), Err(Error::Cancelled)));
    fixture.slow.store(false, Ordering::Relaxed);
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn cancelled_dash_keeps_validated_resources_and_resumes_with_conditional_requests() {
    let fixture = Arc::new(Fixture::new());
    let base = server(fixture.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let id = TaskId::new_v4();
    let task = request(directory.path(), format!("{base}/entry"));
    paused_download(fixture.clone(), task.clone(), id).await;
    assert!(
        directory
            .path()
            .join(format!(".ndm-next-{id}/dash-checkpoint.json"))
            .exists()
    );
    assert!(!directory.path().join("video.mp4").exists());
    let result = download_dash_with_media_tool(
        &task,
        id,
        &DashOptions::default(),
        CancellationToken::new(),
        &tools(),
    )
    .await
    .unwrap();
    assert_eq!(result.tracks, 2);
    assert!(fixture.cache_hits.load(Ordering::Relaxed) >= 1);
    assert!(!directory.path().join(format!(".ndm-next-{id}")).exists());
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn changed_cached_dash_resource_is_not_joined_to_the_previous_transfer() {
    let fixture = Arc::new(Fixture::new());
    let base = server(fixture.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let id = TaskId::new_v4();
    let task = request(directory.path(), format!("{base}/entry"));
    paused_download(fixture.clone(), task.clone(), id).await;
    fixture.version.store(2, Ordering::Relaxed);
    let result = download_dash_with_media_tool(
        &task,
        id,
        &DashOptions::default(),
        CancellationToken::new(),
        &tools(),
    )
    .await;
    assert!(matches!(result, Err(Error::DashRecovery)));
    assert!(!directory.path().join("video.mp4").exists());
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn renewed_dash_address_restarts_owned_resources_and_preserves_other_files() {
    let fixture = Arc::new(Fixture::new());
    let base = server(fixture.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let id = TaskId::new_v4();
    let mut task = request(directory.path(), format!("{base}/entry"));
    paused_download(fixture.clone(), task.clone(), id).await;
    let work = directory.path().join(format!(".ndm-next-{id}"));
    std::fs::write(work.join("notes.txt"), b"preserve unrelated user content").unwrap();
    task.url = format!("{base}/manifests/renewed.mpd?token=renew-token-private");
    let result = download_dash_with_media_tool(
        &task,
        id,
        &DashOptions::default(),
        CancellationToken::new(),
        &tools(),
    )
    .await
    .unwrap();
    assert_eq!(result.tracks, 2);
    assert_eq!(fixture.cache_hits.load(Ordering::Relaxed), 0);
    assert_eq!(
        std::fs::read(work.join("notes.txt")).unwrap(),
        b"preserve unrelated user content"
    );
    assert!(!work.join("dash-checkpoint.json").exists());
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn resume_handles_servers_without_conditional_cache_support() {
    for emit_etag in [false, true] {
        let mut fixture = Fixture::new();
        fixture.emit_etag = emit_etag;
        fixture.ignore_conditionals = true;
        let fixture = Arc::new(fixture);
        let base = server(fixture.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let id = TaskId::new_v4();
        let task = request(directory.path(), format!("{base}/entry"));
        paused_download(fixture.clone(), task.clone(), id).await;
        let result = download_dash_with_media_tool(
            &task,
            id,
            &DashOptions::default(),
            CancellationToken::new(),
            &tools(),
        )
        .await
        .unwrap();
        assert_eq!(result.tracks, 2);
        assert_eq!(fixture.cache_hits.load(Ordering::Relaxed), 0);
        assert_eq!(
            fixture
                .headers
                .lock()
                .unwrap()
                .iter()
                .any(|headers| headers.contains_key("If-None-Match")),
            emit_etag
        );
    }
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn credentials_are_not_sent_to_cross_origin_dash_resources() {
    let media = Arc::new(Fixture::new());
    let media_base = server(media.clone()).await;
    let mut fixture = Fixture::new();
    fixture.manifest = fixture
        .manifest
        .replace("../media/", &format!("{media_base}/media/"));
    let fixture = Arc::new(fixture);
    let base = server(fixture.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let mut task = request(directory.path(), format!("{base}/entry"));
    task.headers
        .insert("authorization".into(), "Bearer private".into());
    task.headers
        .insert("cookie".into(), "session=private".into());
    download_dash_with_media_tool(
        &task,
        TaskId::new_v4(),
        &DashOptions::default(),
        CancellationToken::new(),
        &tools(),
    )
    .await
    .unwrap();
    assert!(
        fixture
            .headers
            .lock()
            .unwrap()
            .iter()
            .any(|headers| headers.contains_key("authorization"))
    );
    assert!(!media.headers.lock().unwrap().is_empty());
    assert!(
        media.headers.lock().unwrap().iter().all(
            |headers| !headers.contains_key("authorization") && !headers.contains_key("cookie")
        )
    );
}
