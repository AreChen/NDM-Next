use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode, header},
    routing::get,
};
use bytes::Bytes;
use ndm_core::{
    DownloadProgress, HttpOptions, NewTask, ProxyOptions, Store, TaskId, TaskState, download_http,
    execute_http,
};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::RwLock};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn missing_suggested_filename_uses_content_disposition_or_url_safely() {
    let dir = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/header", get(|| async {
            ([(header::CONTENT_DISPOSITION, "attachment; filename=\"fallback.bin\"; filename*=UTF-8''%E4%B8%8B%E8%BD%BD.bin")], "hello")
        }))
        .route("/files/from-url.bin", get(|| async { "hello" }));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    for (url_path, expected) in [
        ("header", "下载.bin"),
        ("files/from-url.bin", "from-url.bin"),
    ] {
        let mut req = request(dir.path(), format!("http://{address}/{url_path}"));
        req.suggested_filename = None;
        let output = download_http(
            &req,
            TaskId::new_v4(),
            &HttpOptions::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(output.file_name().unwrap().to_str().unwrap(), expected);
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Ranges,
    IgnoreRangeUnknownLength,
    MalformedRange,
    MalformedSegment,
    DropOnce,
}

#[derive(Clone)]
struct Fixture {
    current: Arc<RwLock<(String, Vec<u8>)>>,
    mode: Mode,
    range_requests: Arc<AtomicUsize>,
    ranges: Arc<Mutex<Vec<String>>>,
    sent_bytes: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    dropped_range: Arc<Mutex<Option<String>>>,
    slow: bool,
}

async fn respond(State(fixture): State<Fixture>, headers: HeaderMap) -> Response<Body> {
    let (etag, data) = fixture.current.read().await.clone();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    if let Some(range) = range {
        fixture.range_requests.fetch_add(1, Ordering::SeqCst);
        fixture.ranges.lock().unwrap().push(range.to_owned());
    }
    let if_range_matches = headers
        .get(header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v == etag);
    let usable_range = matches!(
        fixture.mode,
        Mode::Ranges | Mode::MalformedRange | Mode::MalformedSegment | Mode::DropOnce
    ) && range.is_some()
        && if_range_matches;
    let (start, end) = if usable_range {
        let value = range.unwrap().strip_prefix("bytes=").unwrap();
        let (start, end) = value.split_once('-').unwrap();
        (
            start.parse::<usize>().unwrap(),
            end.parse::<usize>().unwrap(),
        )
    } else {
        (0, data.len() - 1)
    };
    let selected = data[start..=end].to_vec();
    let mut response = Response::builder()
        .status(if usable_range {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::ETAG, etag);
    if usable_range {
        let content_range = if matches!(fixture.mode, Mode::MalformedRange)
            || (matches!(fixture.mode, Mode::MalformedSegment) && range != Some("bytes=0-0"))
        {
            format!("bytes 1-1/{}", data.len())
        } else {
            format!("bytes {start}-{end}/{}", data.len())
        };
        response = response.header(header::CONTENT_RANGE, content_range);
    }
    if !matches!(fixture.mode, Mode::IgnoreRangeUnknownLength) {
        response = response.header(header::CONTENT_LENGTH, selected.len());
    }
    if matches!(fixture.mode, Mode::DropOnce)
        && usable_range
        && range != Some("bytes=0-0")
        && !fixture.dropped.swap(true, Ordering::SeqCst)
    {
        let first = Bytes::copy_from_slice(&selected[..selected.len().min(8192)]);
        *fixture.dropped_range.lock().unwrap() = range.map(str::to_owned);
        let stream = futures_util::stream::unfold(0, move |step| {
            let first = first.clone();
            async move {
                match step {
                    0 => Some((Ok(first), 1)),
                    1 => {
                        tokio::time::sleep(Duration::from_millis(80)).await;
                        Some((
                            Err(std::io::Error::new(
                                std::io::ErrorKind::ConnectionReset,
                                "injected disconnect",
                            )),
                            2,
                        ))
                    }
                    _ => None,
                }
            }
        });
        return response.body(Body::from_stream(stream)).unwrap();
    }
    let chunks: Vec<Bytes> = selected.chunks(8192).map(Bytes::copy_from_slice).collect();
    let delay = fixture.slow;
    let sent_bytes = fixture.sent_bytes.clone();
    let stream = futures_util::stream::unfold(chunks.into_iter(), move |mut chunks| {
        let sent_bytes = sent_bytes.clone();
        async move {
            let chunk = chunks.next()?;
            if delay {
                tokio::time::sleep(Duration::from_millis(8)).await;
            }
            sent_bytes.fetch_add(chunk.len(), Ordering::SeqCst);
            Some((Ok::<_, std::io::Error>(chunk), chunks))
        }
    });
    response.body(Body::from_stream(stream)).unwrap()
}

async fn fixture(mode: Mode, data: Vec<u8>, slow: bool) -> (Fixture, String) {
    let state = Fixture {
        current: Arc::new(RwLock::new(("\"version-1\"".into(), data))),
        mode,
        range_requests: Arc::new(AtomicUsize::new(0)),
        ranges: Arc::new(Mutex::new(Vec::new())),
        sent_bytes: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
        dropped_range: Arc::new(Mutex::new(None)),
        slow,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/file", get(respond))
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (state, format!("http://{address}/file"))
}

fn request(destination: &std::path::Path, url: String) -> NewTask {
    NewTask {
        url,
        destination: destination.to_path_buf(),
        suggested_filename: Some("file.bin".into()),
        source_url: None,
        expected_size: None,
        headers: BTreeMap::new(),
    }
}

fn data(seed: usize) -> Vec<u8> {
    (0..524_288)
        .map(|i| ((i * 17 + seed) % 251) as u8)
        .collect()
}

#[tokio::test]
async fn segmented_download_preserves_existing_target() {
    let dir = TempDir::new().unwrap();
    let expected = data(1);
    let (server, url) = fixture(Mode::Ranges, expected.clone(), false).await;
    std::fs::write(dir.path().join("file.bin"), b"existing").unwrap();
    let output = download_http(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("file.bin")).unwrap(),
        b"existing"
    );
    assert_eq!(std::fs::read(output).unwrap(), expected);
    assert!(server.range_requests.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn ignored_range_and_unknown_length_still_download() {
    let dir = TempDir::new().unwrap();
    let expected = data(2);
    let (_, url) = fixture(Mode::IgnoreRangeUnknownLength, expected.clone(), false).await;
    let output = download_http(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
}

#[tokio::test]
async fn malformed_range_never_corrupts_output() {
    let dir = TempDir::new().unwrap();
    let expected = data(3);
    let (_, url) = fixture(Mode::MalformedRange, expected.clone(), false).await;
    let output = download_http(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
}

#[tokio::test]
async fn oversized_chunked_range_is_rejected_without_filling_disk() {
    use futures_util::StreamExt;

    let dir = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app =
        Router::new().route(
            "/file",
            get(|headers: HeaderMap| async move {
                let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
                if range == Some("bytes=0-0") {
                    return Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_RANGE, "bytes 0-0/1024")
                        .header(header::ETAG, "\"version-1\"")
                        .body(Body::from("x"))
                        .unwrap();
                }
                if range == Some("bytes=0-1023") {
                    let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(
                        Bytes::from(vec![b'x'; 4096]),
                    )])
                    .chain(futures_util::stream::pending());
                    return Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_RANGE, "bytes 0-1023/1024")
                        .header(header::ETAG, "\"version-1\"")
                        .body(Body::from_stream(stream))
                        .unwrap();
                }
                Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let id = TaskId::new_v4();
    let options = HttpOptions {
        max_connections: 1,
        ..Default::default()
    };
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        download_http(
            &request(dir.path(), format!("http://{address}/file")),
            id,
            &options,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("an oversized stream must be rejected without waiting for EOF");
    assert!(result.is_err());
    let segment = dir.path().join(format!(".ndm-next-{id}/segment-0.part"));
    assert!(std::fs::metadata(segment).unwrap().len() <= 1024);
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn oversized_chunked_full_response_respects_known_expected_size() {
    use futures_util::StreamExt;

    let dir = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/file",
        get(|| async move {
            let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from(vec![
                    b'x';
                    4096
                ]))])
            .chain(futures_util::stream::pending());
            Response::builder()
                .status(StatusCode::OK)
                .body(Body::from_stream(stream))
                .unwrap()
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let id = TaskId::new_v4();
    let mut req = request(dir.path(), format!("http://{address}/file"));
    req.expected_size = Some(1024);
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        download_http(&req, id, &HttpOptions::default(), CancellationToken::new()),
    )
    .await
    .expect("an oversized stream must be rejected without waiting for EOF");
    assert!(result.is_err());
    let partial = dir.path().join(format!(".ndm-next-{id}/assembled.part"));
    assert!(std::fs::metadata(partial).unwrap().len() <= 1024);
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn changed_validator_discards_old_partial_content() {
    let dir = TempDir::new().unwrap();
    let (server, url) = fixture(Mode::Ranges, data(4), true).await;
    let req = request(dir.path(), url);
    let id = TaskId::new_v4();
    let cancel = CancellationToken::new();
    let pending = tokio::spawn({
        let req = req.clone();
        let cancel = cancel.clone();
        async move { download_http(&req, id, &HttpOptions::default(), cancel).await }
    });
    let part_dir = dir.path().join(format!(".ndm-next-{id}"));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            assert!(
                !pending.is_finished(),
                "download stopped before sending a partial response"
            );
            if server.sent_bytes.load(Ordering::SeqCst) >= 131_072 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(pending.await.unwrap().is_err());
    let partial_bytes: u64 = std::fs::read_dir(&part_dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("segment-"))
        .map(|e| e.metadata().unwrap().len())
        .sum();
    assert!(
        partial_bytes > 0,
        "paused download should retain bytes already received"
    );

    let replacement = data(5);
    *server.current.write().await = ("\"version-2\"".into(), replacement.clone());
    let output = download_http(&req, id, &HttpOptions::default(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), replacement);
}

#[tokio::test]
async fn changed_link_does_not_mix_old_partial_content_even_if_etag_matches() {
    let dir = TempDir::new().unwrap();
    let (server, url) = fixture(Mode::Ranges, data(7), true).await;
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let id = TaskId::new_v4();
    store
        .prepare(id, &request(dir.path(), format!("{url}?old")))
        .unwrap();
    store.commit(id).unwrap();
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let store = store.clone();
        let cancel = cancel.clone();
        async move {
            execute_http(
                &store,
                id,
                &HttpOptions::default(),
                cancel,
                DownloadProgress::default(),
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while server.sent_bytes.load(Ordering::SeqCst) < 131_072 {
            assert!(!running.is_finished());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(running.await.unwrap().is_err());
    let replacement = data(8);
    *server.current.write().await = ("\"version-1\"".into(), replacement.clone());
    store.update_web_url(id, &format!("{url}?new")).unwrap();
    store.resume(id).unwrap();
    let output = execute_http(
        &store,
        id,
        &HttpOptions::default(),
        CancellationToken::new(),
        DownloadProgress::default(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), replacement);
}

#[tokio::test]
async fn paused_download_resumes_the_existing_segments() {
    let dir = TempDir::new().unwrap();
    let expected = data(6);
    let connections = HttpOptions::default().max_connections;
    let segment_bytes = expected.len() as u64 / connections as u64;
    let (server, url) = fixture(Mode::Ranges, expected.clone(), true).await;
    let req = request(dir.path(), url);
    let id = TaskId::new_v4();
    let part_dir = dir.path().join(format!(".ndm-next-{id}"));
    let cancel = CancellationToken::new();
    let pending = tokio::spawn({
        let req = req.clone();
        let cancel = cancel.clone();
        async move { download_http(&req, id, &HttpOptions::default(), cancel).await }
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !(0..connections).any(|i| {
            std::fs::metadata(part_dir.join(format!("segment-{i}.part")))
                .is_ok_and(|metadata| metadata.len() > 0 && metadata.len() < segment_bytes)
        }) {
            assert!(
                !pending.is_finished(),
                "download finished before it could be paused"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(pending.await.unwrap().is_err());
    let partial_lengths: Vec<_> = (0..connections)
        .map(|i| {
            std::fs::metadata(part_dir.join(format!("segment-{i}.part")))
                .unwrap()
                .len()
        })
        .collect();
    assert!(partial_lengths.iter().any(|&n| n > 0 && n < segment_bytes));
    let old_count = server.ranges.lock().unwrap().len();

    let output = download_http(&req, id, &HttpOptions::default(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
    let ranges = server.ranges.lock().unwrap();
    assert!(
        partial_lengths.iter().enumerate().any(|(i, &length)| {
            length > 0
                && length < segment_bytes
                && ranges[old_count..].contains(&format!(
                    "bytes={}-{}",
                    i as u64 * segment_bytes + length,
                    (i as u64 + 1) * segment_bytes - 1
                ))
        }),
        "resume should request bytes after a saved partial segment"
    );
}

#[tokio::test]
async fn windows_reserved_filename_does_not_break_download() {
    let dir = TempDir::new().unwrap();
    let expected = data(7);
    let (_, url) = fixture(Mode::Ranges, expected.clone(), false).await;
    let mut req = request(dir.path(), url);
    req.suggested_filename = Some("CON.txt".into());
    let output = download_http(
        &req,
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
}

#[tokio::test]
async fn queued_task_executes_and_persists_its_completed_output() {
    let dir = TempDir::new().unwrap();
    let expected = data(8);
    let (_, url) = fixture(Mode::Ranges, expected.clone(), false).await;
    let id = TaskId::new_v4();
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    store.prepare(id, &request(dir.path(), url)).unwrap();
    store.commit(id).unwrap();
    let progress = DownloadProgress::default();
    let output = execute_http(
        &store,
        id,
        &HttpOptions::default(),
        CancellationToken::new(),
        progress.clone(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    assert_eq!(progress.snapshot().downloaded, expected.len() as u64);
    assert_eq!(progress.snapshot().total, Some(expected.len() as u64));
    drop(store);
    let task = Store::open(&dir.path().join("tasks.db"))
        .unwrap()
        .get(id)
        .unwrap()
        .unwrap();
    assert_eq!(task.state, TaskState::Completed);
    assert_eq!(task.output_path.as_deref(), Some(output.as_path()));
    assert_eq!(task.size_verified, Some(true));
}

#[tokio::test]
async fn progress_reports_real_parallel_segment_bytes_and_completion() {
    let dir = TempDir::new().unwrap();
    let expected = data(10);
    let (_, url) = fixture(Mode::Ranges, expected.clone(), true).await;
    let id = TaskId::new_v4();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    store.prepare(id, &request(dir.path(), url)).unwrap();
    store.commit(id).unwrap();
    let progress = DownloadProgress::default();
    let task = {
        let store = store.clone();
        let progress = progress.clone();
        tokio::spawn(async move {
            execute_http(
                &store,
                id,
                &HttpOptions::default(),
                CancellationToken::new(),
                progress,
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = progress.snapshot();
            if snapshot.segments.len() == HttpOptions::default().max_connections
                && snapshot
                    .segments
                    .iter()
                    .filter(|segment| segment.active)
                    .count()
                    > 1
                && snapshot
                    .segments
                    .iter()
                    .any(|segment| segment.downloaded > 0)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("parallel segment progress should become visible");
    task.await.unwrap().unwrap();
    let snapshot = progress.snapshot();
    assert_eq!(
        snapshot.segments.len(),
        HttpOptions::default().max_connections
    );
    assert_eq!(
        snapshot
            .segments
            .iter()
            .map(|segment| segment.downloaded)
            .sum::<u64>(),
        expected.len() as u64
    );
    assert!(
        snapshot
            .segments
            .iter()
            .all(|segment| !segment.active && Some(segment.downloaded) == segment.total)
    );
}

#[tokio::test]
async fn unknown_size_download_is_marked_unverified() {
    let dir = TempDir::new().unwrap();
    let (_, url) = fixture(Mode::IgnoreRangeUnknownLength, data(14), false).await;
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), url)).unwrap();
    store.commit(id).unwrap();
    execute_http(
        &store,
        id,
        &HttpOptions::default(),
        CancellationToken::new(),
        DownloadProgress::default(),
    )
    .await
    .unwrap();
    let task = store.get(id).unwrap().unwrap();
    assert_eq!(task.state, TaskState::Completed);
    assert_eq!(task.size_verified, Some(false));
}

#[tokio::test]
async fn unknown_length_archive_can_follow_a_cross_origin_redirect() {
    let dir = TempDir::new().unwrap();
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_address = target_listener.local_addr().unwrap();
    let leaked_cookie = Arc::new(AtomicBool::new(false));
    let observed = leaked_cookie.clone();
    let target = Router::new().route(
        "/archive",
        get(move |headers: HeaderMap| {
            let observed = observed.clone();
            async move {
                observed.store(headers.contains_key(header::COOKIE), Ordering::SeqCst);
                let chunks = futures_util::stream::iter([Ok::<_, std::io::Error>(
                    Bytes::from_static(b"archive contents"),
                )]);
                Response::builder()
                    .status(StatusCode::OK)
                    .header(
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"repo-main.zip\"",
                    )
                    .body(Body::from_stream(chunks))
                    .unwrap()
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(target_listener, target).await.unwrap();
    });
    let origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_address = origin_listener.local_addr().unwrap();
    let location = format!("http://{target_address}/archive");
    let origin = Router::new().route(
        "/zip",
        get(move || {
            let location = location.clone();
            async move {
                Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, location)
                    .body(Body::empty())
                    .unwrap()
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(origin_listener, origin).await.unwrap();
    });
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    let mut req = request(dir.path(), format!("http://{origin_address}/zip"));
    req.suggested_filename = None;
    req.headers
        .insert("cookie".into(), "session=private".into());
    store.prepare(id, &req).unwrap();
    store.commit(id).unwrap();
    let progress = DownloadProgress::default();
    let output = execute_http(
        &store,
        id,
        &HttpOptions::default(),
        CancellationToken::new(),
        progress.clone(),
    )
    .await
    .unwrap();
    assert_eq!(output.file_name().unwrap(), "repo-main.zip");
    assert_eq!(std::fs::read(output).unwrap(), b"archive contents");
    assert_eq!(progress.snapshot().total, None);
    assert_eq!(store.get(id).unwrap().unwrap().size_verified, Some(false));
    assert!(!leaked_cookie.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancelled_task_stays_paused_until_explicitly_resumed() {
    let dir = TempDir::new().unwrap();
    let expected = data(9);
    let (server, url) = fixture(Mode::Ranges, expected.clone(), true).await;
    let id = TaskId::new_v4();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    store.prepare(id, &request(dir.path(), url)).unwrap();
    store.commit(id).unwrap();
    let cancel = CancellationToken::new();
    let pending = tokio::spawn({
        let store = store.clone();
        let cancel = cancel.clone();
        async move {
            execute_http(
                &store,
                id,
                &HttpOptions::default(),
                cancel,
                DownloadProgress::default(),
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while server.sent_bytes.load(Ordering::SeqCst) < 131_072 {
            assert!(!pending.is_finished());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(pending.await.unwrap().is_err());
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Paused);
    store.recover_interrupted().unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Paused);
    store.resume(id).unwrap();
    let output = execute_http(
        &store,
        id,
        &HttpOptions::default(),
        CancellationToken::new(),
        DownloadProgress::default(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Completed);
}

#[tokio::test]
async fn cross_origin_redirect_does_not_forward_private_headers_or_page_query() {
    let captured = Arc::new(Mutex::new(Vec::<HeaderMap>::new()));
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination_address = destination.local_addr().unwrap();
    let receiver = Router::new().route(
        "/file",
        get({
            let captured = captured.clone();
            move |headers: HeaderMap| {
                let captured = captured.clone();
                async move {
                    captured.lock().unwrap().push(headers);
                    "download"
                }
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(destination, receiver).await.unwrap();
    });
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_address = origin.local_addr().unwrap();
    let redirect_url = format!("http://{destination_address}/file");
    let redirector = Router::new().route(
        "/redirect",
        get(move || {
            let redirect_url = redirect_url.clone();
            async move { (StatusCode::FOUND, [(header::LOCATION, redirect_url)]) }
        }),
    );
    tokio::spawn(async move {
        axum::serve(origin, redirector).await.unwrap();
    });

    let dir = TempDir::new().unwrap();
    let mut req = request(
        dir.path(),
        format!("http://{origin_address}/redirect?token=private-url"),
    );
    req.source_url = Some(format!("http://{origin_address}/page?token=private-page"));
    req.headers
        .insert("Cookie".into(), "session=private-cookie".into());
    req.headers
        .insert("Authorization".into(), "Bearer private-token".into());
    req.headers
        .insert("X-Api-Key".into(), "private-api-key".into());
    let output = download_http(
        &req,
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"download");
    let captured = captured.lock().unwrap();
    assert!(!captured.is_empty());
    for headers in captured.iter() {
        assert!(!headers.contains_key(header::COOKIE));
        assert!(!headers.contains_key(header::AUTHORIZATION));
        assert!(!headers.contains_key("x-api-key"));
        if let Some(referer) = headers.get(header::REFERER) {
            let referer = referer.to_str().unwrap();
            assert!(!referer.contains("private-"));
        }
    }
}

#[tokio::test]
async fn invalid_segment_range_falls_back_to_an_uncorrupted_single_download() {
    let dir = TempDir::new().unwrap();
    let (_, url) = fixture(Mode::MalformedSegment, data(10), false).await;
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), url)).unwrap();
    store.commit(id).unwrap();
    let output = execute_http(
        &store,
        id,
        &HttpOptions::default(),
        CancellationToken::new(),
        DownloadProgress::default(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), data(10));
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Completed);
}

#[tokio::test]
async fn unexpected_file_size_cannot_be_reported_as_completed() {
    let dir = TempDir::new().unwrap();
    let (_, url) = fixture(Mode::Ranges, data(11), false).await;
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    let mut req = request(dir.path(), url);
    req.expected_size = Some(42);
    store.prepare(id, &req).unwrap();
    store.commit(id).unwrap();
    assert!(
        execute_http(
            &store,
            id,
            &HttpOptions::default(),
            CancellationToken::new(),
            DownloadProgress::default()
        )
        .await
        .is_err()
    );
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Failed);
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn configured_speed_limit_applies_to_the_whole_download() {
    let dir = TempDir::new().unwrap();
    let expected = vec![b'x'; 32_768];
    let (_, url) = fixture(Mode::Ranges, expected.clone(), false).await;
    let options = HttpOptions {
        max_connections: 1,
        max_bytes_per_second: Some(65_536),
        ..Default::default()
    };
    let started = std::time::Instant::now();
    let output = download_http(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &options,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
    assert!(
        started.elapsed() >= Duration::from_millis(420),
        "speed limit allowed the file to arrive too quickly"
    );
}

#[tokio::test]
async fn authenticated_proxy_is_used_for_http_downloads_without_exposing_credentials_in_debug() {
    let dir = TempDir::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let proxy = Router::new().fallback(get(move |headers: HeaderMap| {
        let observed = observed.clone();
        async move {
            if headers
                .get(header::PROXY_AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                != Some("Basic cHJveHl1c2VyOnByb3h5cGFzcw==")
            {
                return (
                    StatusCode::PROXY_AUTHENTICATION_REQUIRED,
                    "authentication required",
                );
            }
            observed.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, "proxied content")
        }
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, proxy).await.unwrap();
    });
    let options = HttpOptions {
        proxy: Some(ProxyOptions {
            url: format!("http://{address}"),
            username: Some("proxyuser".into()),
            password: Some("proxypass".into()),
        }),
        ..Default::default()
    };
    let output = download_http(
        &request(dir.path(), "http://not-resolvable.invalid/file".into()),
        TaskId::new_v4(),
        &options,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"proxied content");
    assert!(calls.load(Ordering::SeqCst) >= 1);
    assert!(!format!("{options:?}").contains("proxypass"));
}

#[tokio::test]
async fn changing_redirect_destinations_cannot_mix_different_files_with_same_etag() {
    let dir = TempDir::new().unwrap();
    let expected = data(12);
    let (_, url_a) = fixture(Mode::Ranges, expected.clone(), false).await;
    let (_, url_b) = fixture(Mode::Ranges, data(13), false).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let redirector = Router::new().route(
        "/file",
        get(move |headers: HeaderMap| {
            let counter = counter.clone();
            let a = url_a.clone();
            let b = url_b.clone();
            async move {
                let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
                let destination = match range {
                    None | Some("bytes=0-0") => a,
                    Some(_) if counter.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) => b,
                    Some(_) => a,
                };
                (StatusCode::FOUND, [(header::LOCATION, destination)])
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, redirector).await.unwrap();
    });
    let output = download_http(
        &request(dir.path(), format!("http://{address}/file")),
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(
        std::fs::read(output).unwrap() == expected,
        "redirect changes must not mix different files"
    );
}

#[tokio::test]
async fn temporary_server_errors_are_retried_without_creating_extra_files() {
    let dir = TempDir::new().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/file",
        get({
            let requests = requests.clone();
            move || {
                let requests = requests.clone();
                async move {
                    if requests.fetch_add(1, Ordering::SeqCst) < 2 {
                        (StatusCode::SERVICE_UNAVAILABLE, "try again")
                    } else {
                        (StatusCode::OK, "recovered")
                    }
                }
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let output = download_http(
        &request(dir.path(), format!("http://{address}/file")),
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"recovered");
    assert_eq!(requests.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn interrupted_segment_is_retried_from_its_saved_offset() {
    let dir = TempDir::new().unwrap();
    let expected = data(15);
    let (server, url) = fixture(Mode::DropOnce, expected.clone(), false).await;
    let output = download_http(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(
        std::fs::read(output).unwrap() == expected,
        "retry must preserve all bytes"
    );
    assert!(server.range_requests.load(Ordering::SeqCst) >= 6);
    let failed_range = server.dropped_range.lock().unwrap().clone().unwrap();
    let (start, end) = failed_range
        .strip_prefix("bytes=")
        .unwrap()
        .split_once('-')
        .unwrap();
    let expected_retry = format!("bytes={}-{end}", start.parse::<u64>().unwrap() + 8192);
    assert!(
        server.ranges.lock().unwrap().contains(&expected_retry),
        "retry should start after the saved bytes"
    );
}

#[tokio::test]
async fn empty_file_download_succeeds_when_range_probe_is_unsatisfiable() {
    let dir = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/file",
        get(|headers: HeaderMap| async move {
            if headers.contains_key(header::RANGE) {
                (StatusCode::RANGE_NOT_SATISFIABLE, "")
            } else {
                (StatusCode::OK, "")
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let output = download_http(
        &request(dir.path(), format!("http://{address}/file")),
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::metadata(output).unwrap().len(), 0);
}

#[cfg(windows)]
#[tokio::test]
async fn cleanup_failure_does_not_mark_a_published_file_as_failed() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = TempDir::new().unwrap();
    let expected = data(16);
    let (_, url) = fixture(Mode::Ranges, expected.clone(), true).await;
    let id = TaskId::new_v4();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    store.prepare(id, &request(dir.path(), url)).unwrap();
    store.commit(id).unwrap();
    let pending = tokio::spawn({
        let store = store.clone();
        async move {
            execute_http(
                &store,
                id,
                &HttpOptions::default(),
                CancellationToken::new(),
                DownloadProgress::default(),
            )
            .await
        }
    });
    let manifest = dir
        .path()
        .join(format!(".ndm-next-{id}"))
        .join("resume.bin");
    let lock = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&manifest)
            {
                break file;
            }
            assert!(
                !pending.is_finished(),
                "download completed before the fixture locked its metadata"
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    let output = pending.await.unwrap().unwrap();
    drop(lock);
    assert!(std::fs::read(output).unwrap() == expected);
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Completed);
}
