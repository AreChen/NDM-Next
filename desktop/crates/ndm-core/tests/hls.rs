use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::Response,
    routing::get,
};
use ndm_core::{
    HlsOptions, NewTask, Scheduler, SchedulerOptions, Store, TaskId, TaskState, download_hls,
    inspect_hls,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Fixture {
    root: PathBuf,
    missing: bool,
}

async fn serve(State(fixture): State<Arc<Fixture>>, AxumPath(path): AxumPath<String>) -> Response {
    if path == "long-vod.m3u8" {
        let mut playlist = String::from(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"video/init.mp4\"\n",
        );
        for index in 0..129 {
            // Repeat the short fixture as separate timeline epochs. This keeps
            // the repository small while testing a real >128-segment remux.
            if index > 0 && index % 3 == 0 {
                playlist.push_str("#EXT-X-DISCONTINUITY\n");
            }
            playlist.push_str(&format!("#EXTINF:1.0,\nvideo/seg{:03}.m4s\n", index % 3));
        }
        playlist.push_str("#EXT-X-ENDLIST\n");
        return Response::new(Body::from(playlist));
    }
    if fixture.missing && (path.ends_with("seg001.ts") || path == "mpeg2/index1.ts") {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    }
    if path == "master.m3u8" {
        let master = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio_en/index.m3u8\"\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Japanese\",LANGUAGE=\"ja\",DEFAULT=NO,AUTOSELECT=YES,URI=\"audio_ja/index.m3u8\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=100000,AUDIO=\"aud\"\nvideo/index.m3u8\n";
        return Response::new(Body::from(master));
    }
    if path == "master-mpeg2.m3u8" {
        let master = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio_en/index.m3u8\"\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Japanese\",LANGUAGE=\"ja\",DEFAULT=NO,AUTOSELECT=YES,URI=\"audio_ja/index.m3u8\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=100000,AUDIO=\"aud\"\nmpeg2/index.m3u8\n";
        return Response::new(Body::from(master));
    }
    if path.split('/').any(|part| part == "..") {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .unwrap();
    }
    match tokio::fs::read(fixture.root.join(path)).await {
        Ok(bytes) => Response::new(Body::from(bytes)),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
    }
}

async fn server(missing: bool) -> String {
    let fixture = Arc::new(Fixture {
        root: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls"),
        missing,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/{*path}", get(serve))
        .with_state(fixture);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{address}")
}

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

fn save_for_external_verification(path: &Path, name: &str) {
    if let Some(directory) = std::env::var_os("NDM_HLS_VERIFY_OUTPUTS") {
        std::fs::copy(path, Path::new(&directory).join(name)).unwrap();
    }
}

#[tokio::test]
async fn vod_over_default_queue_limit_downloads_and_merges() {
    let base = server(false).await;
    let dir = TempDir::new().unwrap();
    let result = download_hls(
        &request(dir.path(), format!("{base}/long-vod.m3u8")),
        TaskId::new_v4(),
        &HlsOptions::default(),
        CancellationToken::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let output = std::fs::read(result.path).unwrap();
    assert_eq!(&output[4..8], b"ftyp");
    assert_eq!(result.tracks, 1);
    assert!(output.len() > 128 * 1024);
}

#[tokio::test]
async fn vod_ts_fmp4_and_aes_are_muxed_to_mp4() {
    let base = server(false).await;
    for (playlist, tracks) in [
        ("ts/index.m3u8", 2),
        ("fmp4/index.m3u8", 2),
        ("aes/index.m3u8", 2),
    ] {
        let dir = TempDir::new().unwrap();
        let result = download_hls(
            &request(dir.path(), format!("{base}/{playlist}")),
            TaskId::new_v4(),
            &HlsOptions::default(),
            CancellationToken::new(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        save_for_external_verification(
            &result.path,
            &playlist.replace('/', "-").replace(".m3u8", ".mp4"),
        );
        let output = std::fs::read(result.path).unwrap();
        assert_eq!(&output[4..8], b"ftyp");
        assert!(result.tracks >= tracks);
    }
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn unsupported_mpeg2_is_remuxed_by_local_media_tool() {
    let base = server(false).await;
    let tool = std::env::var_os("NDM_TEST_FFMPEG_DIR").expect("NDM_TEST_FFMPEG_DIR is required");
    for fixture in ["mpeg2", "mpeg2-aes"] {
        let dir = TempDir::new().unwrap();
        let id = TaskId::new_v4();
        let result = ndm_core::download_hls_with_media_tool(
            &request(dir.path(), format!("{base}/{fixture}/index.m3u8")),
            id,
            &HlsOptions::default(),
            CancellationToken::new(),
            CancellationToken::new(),
            Path::new(&tool),
        )
        .await
        .unwrap();
        let bytes = std::fs::read(&result.path).unwrap();
        assert_eq!(&bytes[..4], &[0x1a, 0x45, 0xdf, 0xa3]);
        assert_eq!(result.path.extension().unwrap(), "mkv");
        assert_eq!(result.tracks, 2);
        assert!(!dir.path().join(format!(".ndm-next-{id}")).exists());
        save_for_external_verification(&result.path, &format!("{fixture}-fallback.mkv"));
    }
}

#[tokio::test]
#[ignore = "requires NDM_TEST_FFMPEG_DIR containing the audited media tools"]
async fn fallback_preserves_selected_external_audio_and_rejects_missing_media() {
    let tool = std::env::var_os("NDM_TEST_FFMPEG_DIR").expect("NDM_TEST_FFMPEG_DIR is required");
    let base = server(false).await;
    let dir = TempDir::new().unwrap();
    let options = HlsOptions {
        audio_names: vec!["English".into(), "Japanese".into()],
        ..Default::default()
    };
    let output = ndm_core::download_hls_with_media_tool(
        &request(dir.path(), format!("{base}/master-mpeg2.m3u8")),
        TaskId::new_v4(),
        &options,
        CancellationToken::new(),
        CancellationToken::new(),
        Path::new(&tool),
    )
    .await
    .unwrap();
    assert_eq!(output.tracks, 3);
    let probe = Path::new(&tool).join(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let metadata = std::process::Command::new(probe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream_tags=language",
            "-of",
            "json",
        ])
        .arg(&output.path)
        .output()
        .unwrap();
    assert!(metadata.status.success());
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
    assert_eq!(metadata["streams"][0]["tags"]["language"], "eng");
    assert_eq!(metadata["streams"][1]["tags"]["language"], "jpn");
    save_for_external_verification(&output.path, "mpeg2-multiaudio.mp4");

    let missing = server(true).await;
    let another = TempDir::new().unwrap();
    let result = ndm_core::download_hls_with_media_tool(
        &request(another.path(), format!("{missing}/mpeg2/index.m3u8")),
        TaskId::new_v4(),
        &HlsOptions::default(),
        CancellationToken::new(),
        CancellationToken::new(),
        Path::new(&tool),
    )
    .await;
    assert!(result.is_err());
    assert!(!another.path().join("video.mp4").exists());
    assert!(!another.path().join("video.mkv").exists());
}

#[tokio::test]
async fn scheduler_runs_hls_tasks_in_its_media_worker() {
    let base = server(false).await;
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let id = TaskId::new_v4();
    store
        .prepare(id, &request(dir.path(), format!("{base}/ts/index.m3u8")))
        .unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if store
                .get(id)
                .unwrap()
                .is_some_and(|task| task.state == TaskState::Completed)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(dir.path().join("video.mp4").exists());
    scheduler.shutdown().await;
}

#[tokio::test]
async fn hls_output_uses_mp4_extension_even_when_playlist_name_is_suggested() {
    let base = server(false).await;
    let dir = TempDir::new().unwrap();
    let mut task = request(dir.path(), format!("{base}/ts/index.m3u8"));
    task.suggested_filename = Some("index.m3u8".into());
    let result = download_hls(
        &task,
        TaskId::new_v4(),
        &HlsOptions::default(),
        CancellationToken::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.path.file_name().unwrap(), "index.mp4");
}

#[tokio::test]
async fn master_playlist_selected_external_audio_tracks_are_preserved() {
    let base = server(false).await;
    let dir = TempDir::new().unwrap();
    let options = HlsOptions {
        audio_names: vec!["English".into(), "Japanese".into()],
        ..Default::default()
    };
    let result = download_hls(
        &request(dir.path(), format!("{base}/master.m3u8")),
        TaskId::new_v4(),
        &options,
        CancellationToken::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.tracks, 3);
    save_for_external_verification(&result.path, "external-audio.mp4");
    let output = std::fs::read(result.path).unwrap();
    assert_eq!(&output[4..8], b"ftyp");
}

#[tokio::test]
async fn master_playlist_exposes_quality_and_audio_choices() {
    let base = server(false).await;
    let dir = TempDir::new().unwrap();
    let task = request(dir.path(), format!("{base}/master.m3u8"));
    let catalog = inspect_hls(&task, CancellationToken::new()).await.unwrap();
    assert!(!catalog.direct_media);
    assert_eq!(catalog.variants.len(), 1);
    assert_eq!(catalog.variants[0].bandwidth, 100000);
    assert_eq!(
        catalog.variants[0]
            .audio_tracks
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["English", "Japanese"]
    );
    let invalid = HlsOptions {
        variant_index: Some(2),
        ..Default::default()
    };
    assert!(
        download_hls(
            &task,
            TaskId::new_v4(),
            &invalid,
            CancellationToken::new(),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn missing_hls_segment_does_not_publish_a_completed_file() {
    let base = server(true).await;
    let dir = TempDir::new().unwrap();
    let result = download_hls(
        &request(dir.path(), format!("{base}/ts/index.m3u8")),
        TaskId::new_v4(),
        &HlsOptions::default(),
        CancellationToken::new(),
        CancellationToken::new(),
    )
    .await;
    assert!(result.is_err());
    assert!(!dir.path().join("video.mp4").exists());
}

#[tokio::test]
async fn interrupted_vod_recovers_checked_partial_media() {
    for playlist in ["ts", "aes"] {
        let missing = Arc::new(AtomicBool::new(true));
        let changed_key = Arc::new(AtomicBool::new(false));
        let fixture = Arc::new(Fixture {
            root: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls"),
            missing: false,
        });
        let handler = {
            let missing = missing.clone();
            let changed_key = changed_key.clone();
            move |AxumPath(path): AxumPath<String>| {
                let fixture = fixture.clone();
                let missing = missing.clone();
                let changed_key = changed_key.clone();
                async move {
                    if path.ends_with("seg002.ts") && missing.load(Ordering::SeqCst) {
                        return Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Body::empty())
                            .unwrap();
                    }
                    if path.ends_with("key.bin") && changed_key.load(Ordering::SeqCst) {
                        return Response::new(Body::from(vec![42u8; 16]));
                    }
                    serve(State(fixture), AxumPath(path)).await
                }
            }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/{*path}", get(handler)))
                .await
                .unwrap();
        });
        let dir = TempDir::new().unwrap();
        let id = TaskId::new_v4();
        let task = request(
            dir.path(),
            format!("http://{address}/{playlist}/index.m3u8"),
        );
        let options = HlsOptions::default();
        let run = || {
            download_hls(
                &task,
                id,
                &options,
                CancellationToken::new(),
                CancellationToken::new(),
            )
        };
        assert!(run().await.is_err());
        assert!(!dir.path().join("video.mp4").exists());
        let work = dir.path().join(format!(".ndm-next-{id}"));
        let checkpoint = work.join("hls-checkpoint.sqlite");
        let partial = work.join("hls-output.mp4.hls-partial");
        assert!(
            checkpoint.exists(),
            "{playlist}: checkpoint must survive failure"
        );
        assert!(
            partial.exists(),
            "{playlist}: partial output must survive failure"
        );
        let database = rusqlite::Connection::open(&checkpoint).unwrap();
        let bytes: Vec<u8> = database
            .query_row("SELECT data FROM checkpoint WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let checkpoint = hls_engine::EngineCheckpoint::from_bytes(&bytes).unwrap();
        assert!(
            checkpoint.bytes_written() > 0,
            "{playlist}: expected committed data"
        );
        drop(database);
        missing.store(false, Ordering::SeqCst);
        let original = std::fs::read(&partial).unwrap();
        let mut damaged = original.clone();
        damaged[12] ^= 1;
        std::fs::write(&partial, &damaged).unwrap();
        assert!(
            matches!(run().await, Err(ndm_core::Error::HlsRecovery)),
            "{playlist}: corrupted partial must be rejected as a recovery conflict"
        );
        assert_eq!(std::fs::read(&partial).unwrap(), damaged);
        std::fs::write(&partial, &original).unwrap();
        if playlist == "aes" {
            changed_key.store(true, Ordering::SeqCst);
            assert!(
                matches!(run().await, Err(ndm_core::Error::HlsRecovery)),
                "changed encryption keys must be rejected as a recovery conflict"
            );
            assert_eq!(std::fs::read(&partial).unwrap(), original);
            changed_key.store(false, Ordering::SeqCst);
        }
        let output = run().await.unwrap();
        save_for_external_verification(&output.path, &format!("{playlist}-recovered.mp4"));
        let bytes = std::fs::read(output.path).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(
            !work.exists(),
            "{playlist}: completed temporary files should be cleaned"
        );
    }
}

#[tokio::test]
async fn live_stream_can_stop_and_save_recorded_media() {
    let segments_seen = Arc::new(AtomicUsize::new(0));
    let fixture = Arc::new(Fixture {
        root: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls"),
        missing: false,
    });
    let handler = {
        let segments_seen = segments_seen.clone();
        move |AxumPath(path): AxumPath<String>| {
            let fixture = fixture.clone();
            let segments_seen = segments_seen.clone();
            async move {
                if path == "ts/index.m3u8" {
                    let content = tokio::fs::read_to_string(fixture.root.join(&path))
                        .await
                        .unwrap();
                    return Response::new(Body::from(content.replace("#EXT-X-ENDLIST", "")));
                }
                if path.ends_with(".ts") {
                    segments_seen.fetch_add(1, Ordering::SeqCst);
                }
                serve(State(fixture), AxumPath(path)).await
            }
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/{*path}", get(handler)))
            .await
            .unwrap();
    });
    let dir = TempDir::new().unwrap();
    let id = TaskId::new_v4();
    let stop = CancellationToken::new();
    let stop_when_recorded = stop.clone();
    let stopper = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if segments_seen.load(Ordering::SeqCst) >= 2 {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    stop_when_recorded.cancel();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
    });
    let result = tokio::time::timeout(
        Duration::from_secs(12),
        download_hls(
            &request(dir.path(), format!("http://{address}/ts/index.m3u8")),
            id,
            &HlsOptions::default(),
            CancellationToken::new(),
            stop,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    stopper.await.unwrap();
    assert!(result.stopped);
    save_for_external_verification(&result.path, "live-stopped.mp4");
    assert!(std::fs::metadata(result.path).unwrap().len() > 1000);
}

#[tokio::test]
async fn interrupted_live_stream_resumes_if_its_media_window_is_still_available() {
    let ended = Arc::new(AtomicBool::new(false));
    let fixture = Arc::new(Fixture {
        root: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls"),
        missing: false,
    });
    let handler = {
        let ended = ended.clone();
        move |AxumPath(path): AxumPath<String>| {
            let fixture = fixture.clone();
            let ended = ended.clone();
            async move {
                if path == "ts/index.m3u8" {
                    let content = tokio::fs::read_to_string(fixture.root.join(&path))
                        .await
                        .unwrap();
                    return Response::new(Body::from(if ended.load(Ordering::SeqCst) {
                        content
                    } else {
                        content.replace("#EXT-X-ENDLIST", "")
                    }));
                }
                serve(State(fixture), AxumPath(path)).await
            }
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/{*path}", get(handler)))
            .await
            .unwrap();
    });
    let dir = TempDir::new().unwrap();
    let id = TaskId::new_v4();
    let task = request(dir.path(), format!("http://{address}/ts/index.m3u8"));
    let cancel = CancellationToken::new();
    let work = dir.path().join(format!(".ndm-next-{id}"));
    let options = HlsOptions::default();
    let worker = download_hls(
        &task,
        id,
        &options,
        cancel.clone(),
        CancellationToken::new(),
    );
    let stop_after_checkpoint = async {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let ready = rusqlite::Connection::open_with_flags(
                    work.join("hls-checkpoint.sqlite"),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .ok()
                .and_then(|db| {
                    db.query_row("SELECT data FROM checkpoint WHERE id = 1", [], |row| {
                        row.get::<_, Vec<u8>>(0)
                    })
                    .ok()
                })
                .and_then(|bytes| hls_engine::EngineCheckpoint::from_bytes(&bytes).ok())
                .is_some_and(|checkpoint| checkpoint.bytes_written() > 0);
                if ready {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(worker, stop_after_checkpoint);
    assert!(result.is_err());
    ended.store(true, Ordering::SeqCst);
    let output = download_hls(
        &task,
        id,
        &HlsOptions::default(),
        CancellationToken::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(std::fs::metadata(output.path).unwrap().len() > 1000);
    assert!(!work.exists());
}

#[tokio::test]
async fn live_playlist_is_refreshed_until_endlist() {
    let count = Arc::new(AtomicUsize::new(0));
    let fixture = Arc::new(Fixture {
        root: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls"),
        missing: false,
    });
    let handler = {
        let count = count.clone();
        move |AxumPath(path): AxumPath<String>| {
            let fixture = fixture.clone();
            let count = count.clone();
            async move {
                if path == "ts/index.m3u8" {
                    let content = tokio::fs::read_to_string(fixture.root.join(&path))
                        .await
                        .unwrap();
                    if count.fetch_add(1, Ordering::SeqCst) < 2 {
                        return Response::new(Body::from(content.replace("#EXT-X-ENDLIST", "")));
                    }
                    return Response::new(Body::from(content));
                }
                serve(State(fixture), AxumPath(path)).await
            }
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/{*path}", get(handler)))
            .await
            .unwrap();
    });
    let dir = TempDir::new().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        download_hls(
            &request(dir.path(), format!("http://{address}/ts/index.m3u8")),
            TaskId::new_v4(),
            &HlsOptions::default(),
            CancellationToken::new(),
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!result.stopped);
    assert!(count.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn cross_origin_segments_do_not_receive_browser_credentials() {
    let leaked = Arc::new(AtomicBool::new(false));
    let fixture = Arc::new(Fixture {
        root: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls"),
        missing: false,
    });
    let external_handler = {
        let leaked = leaked.clone();
        move |headers: axum::http::HeaderMap, AxumPath(path): AxumPath<String>| {
            let fixture = fixture.clone();
            let leaked = leaked.clone();
            async move {
                if headers.contains_key("cookie")
                    || headers.contains_key("authorization")
                    || headers.contains_key("x-private")
                {
                    leaked.store(true, Ordering::SeqCst);
                }
                serve(State(fixture), AxumPath(path)).await
            }
        }
    };
    let external = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let external_address = external.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            external,
            Router::new().route("/{*path}", get(external_handler)),
        )
        .await
        .unwrap();
    });
    let original_saw_credentials = Arc::new(AtomicBool::new(false));
    let handler = {
        let original_saw_credentials = original_saw_credentials.clone();
        move |headers: axum::http::HeaderMap| {
            let original_saw_credentials = original_saw_credentials.clone();
            async move {
                if headers.contains_key("cookie") && headers.contains_key("authorization") {
                    original_saw_credentials.store(true, Ordering::SeqCst);
                }
                let content = tokio::fs::read_to_string(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls/ts/index.m3u8"),
                )
                .await
                .unwrap();
                content.replace("seg", &format!("http://{external_address}/ts/seg"))
            }
        }
    };
    let original = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let original_address = original.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(original, Router::new().route("/index.m3u8", get(handler)))
            .await
            .unwrap();
    });
    let dir = TempDir::new().unwrap();
    let mut task = request(dir.path(), format!("http://{original_address}/index.m3u8"));
    task.headers
        .insert("Cookie".into(), "session=private".into());
    task.headers
        .insert("Authorization".into(), "Bearer private".into());
    task.headers.insert("X-Private".into(), "private".into());
    download_hls(
        &task,
        TaskId::new_v4(),
        &HlsOptions::default(),
        CancellationToken::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(original_saw_credentials.load(Ordering::SeqCst));
    assert!(!leaked.load(Ordering::SeqCst));
}
