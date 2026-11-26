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
    DownloadFailure, DownloadProgress, Error, HttpOptions, HttpStage, NewTask, ProxyMode,
    ProxyOptions, ProxyRoute, Store, TaskId, TaskState, download_http, execute_http,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::RwLock};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn running_task_publishes_attachment_name_before_the_body_arrives() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/download?filename=hint.zip",
        listener.local_addr().unwrap()
    );
    let app = Router::new().route(
        "/download",
        get({
            let gate = gate.clone();
            move || {
                let gate = gate.clone();
                async move {
                    Response::builder()
                        .header(
                            header::CONTENT_DISPOSITION,
                            "attachment; filename=actual.zip",
                        )
                        .body(Body::from_stream(futures_util::stream::once(async move {
                            let _permit = gate.acquire().await.unwrap();
                            Ok::<_, std::io::Error>(Bytes::from_static(b"content"))
                        })))
                        .unwrap()
                }
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let id = TaskId::new_v4();
    let mut req = request(dir.path(), url);
    req.suggested_filename = None;
    store.prepare(id, &req).unwrap();
    store.commit(id).unwrap();
    let run = tokio::spawn({
        let store = store.clone();
        async move {
            execute_http(
                &store,
                id,
                &Default::default(),
                CancellationToken::new(),
                Default::default(),
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let task = store.get(id).unwrap().unwrap();
            if task.engine_title.as_deref() == Some("actual.zip") {
                assert_eq!(task.state, TaskState::Running);
                assert!(task.output_path.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    gate.add_permits(1);
    assert_eq!(
        run.await.unwrap().unwrap().file_name().unwrap(),
        "actual.zip"
    );
}

#[tokio::test]
async fn digest_bound_ranges_handle_rotating_urls_and_reject_corrupt_bytes() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ndm_core::HttpRangeMode;
    for (corrupt, fallback) in [(false, false), (true, false), (false, true), (true, true)] {
        let dir = TempDir::new().unwrap();
        let expected = Arc::new(data(3));
        let repr = format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(&*expected)));
        let arrivals = Arc::new(AtomicUsize::new(0));
        let first_pair = Arc::new(tokio::sync::Barrier::new(2));
        let tickets = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/repo",
                get(move || {
                    let ticket = tickets.fetch_add(1, Ordering::SeqCst);
                    async move {
                        (
                            StatusCode::FOUND,
                            [(header::LOCATION, format!("/blob?sig={ticket}"))],
                        )
                    }
                }),
            )
            .route(
                "/blob",
                get({
                    let arrivals = arrivals.clone();
                    let expected = expected.clone();
                    move |headers: HeaderMap| {
                        let arrivals = arrivals.clone();
                        let first_pair = first_pair.clone();
                        let expected = expected.clone();
                        let repr = repr.clone();
                        async move {
                            let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
                            assert!(!headers.contains_key(header::IF_RANGE));
                            let n = expected.len();
                            if range == Some("bytes=0-0") {
                                return Response::builder()
                                    .status(206)
                                    .header("repr-digest", repr)
                                    .header(header::CONTENT_RANGE, format!("bytes 0-0/{n}"))
                                    .body(Body::from(vec![expected[0]]))
                                    .unwrap();
                            }
                            if let Some(range) = range {
                                // Neither of the first two range handlers can respond until
                                // both have arrived, independently of runner scheduling.
                                if arrivals.fetch_add(1, Ordering::SeqCst) < 2 {
                                    first_pair.wait().await;
                                }
                                if fallback {
                                    return Response::builder()
                                        .status(416)
                                        .body(Body::empty())
                                        .unwrap();
                                }
                                let (start, end) = range[6..].split_once('-').unwrap();
                                let (start, end) = (
                                    start.parse::<usize>().unwrap(),
                                    end.parse::<usize>().unwrap(),
                                );
                                let mut part = expected[start..=end].to_vec();
                                if corrupt {
                                    part[0] ^= 1;
                                }
                                return Response::builder()
                                    .status(206)
                                    .header(
                                        header::CONTENT_RANGE,
                                        format!("bytes {start}-{end}/{n}"),
                                    )
                                    .body(Body::from(part))
                                    .unwrap();
                            }
                            let mut full = (*expected).clone();
                            if corrupt {
                                full[0] ^= 1;
                            }
                            Response::builder().body(Body::from(full)).unwrap()
                        }
                    }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/repo?FilePath=vae%2Fmodel.safetensors",
            listener.local_addr().unwrap()
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let id = TaskId::new_v4();
        let mut req = request(dir.path(), url);
        req.suggested_filename = None;
        // A rotating redirect must still reuse a part when the repository digest
        // matches. A different revision of the same size must fetch every part.
        if !fallback {
            let cache = dir.path().join(format!(".ndm-next-{id}"));
            std::fs::create_dir(&cache).unwrap();
            let digest: [u8; 32] = if corrupt {
                [0; 32]
            } else {
                Sha256::digest(&*expected).into()
            };
            let manifest = serde_json::json!({"validator":{"sha256":digest},"resource_url":req.url,"total":expected.len(),"connections":4});
            std::fs::write(
                cache.join("resume.bin"),
                ndm_core::protect_user_data(&serde_json::to_vec(&manifest).unwrap()).unwrap(),
            )
            .unwrap();
            std::fs::write(
                cache.join("segment-0.part"),
                &expected[..expected.len() / 4],
            )
            .unwrap();
        }
        let store = Store::open(&dir.path().join("tasks.db")).unwrap();
        store.prepare(id, &req).unwrap();
        store.commit(id).unwrap();
        let progress = DownloadProgress::default();
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            execute_http(
                &store,
                id,
                &HttpOptions {
                    max_connections: 4,
                    ..Default::default()
                },
                CancellationToken::new(),
                progress.clone(),
            ),
        )
        .await
        .expect("two segment requests must overlap");
        assert!(arrivals.load(Ordering::SeqCst) >= 2);
        if !fallback {
            let reused = if corrupt { 0 } else { expected.len() / 4 };
            assert_eq!(
                progress.snapshot().transferred,
                (expected.len() - reused) as u64
            );
        }
        assert_eq!(
            progress.snapshot().http.unwrap().range_mode,
            Some(if fallback {
                HttpRangeMode::SegmentFallback
            } else {
                HttpRangeMode::VerifiedSha256
            })
        );
        if fallback {
            let evidence = progress
                .snapshot()
                .http
                .unwrap()
                .segment_failure
                .expect("retain failed range after the full transfer");
            assert_eq!(evidence.http_status, Some(416));
            assert!(evidence.index < 4 && evidence.start <= evidence.end);
            assert_eq!(evidence.expected_total, expected.len() as u64);
        }
        if corrupt {
            assert!(matches!(result, Err(Error::ChecksumMismatch)));
            assert_eq!(progress.snapshot().downloaded, 0);
            assert_eq!(
                progress.snapshot().http.unwrap().stage,
                HttpStage::IntegrityCheck
            );
            assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Failed);
            assert!(!dir.path().join("model.safetensors").exists());
            assert!(
                !dir.path()
                    .join(format!(".ndm-next-{id}/resume.bin"))
                    .exists()
            );
        } else {
            let output = result.unwrap();
            assert_eq!(output.file_name().unwrap(), "model.safetensors");
            assert_eq!(std::fs::read(output).unwrap(), *expected);
        }
    }
}

fn receiving_ranges(progress: &DownloadProgress) -> usize {
    progress
        .snapshot()
        .segments
        .iter()
        .filter(|segment| segment.active && segment.downloaded > 0)
        .count()
}

#[tokio::test]
#[ignore = "downloads public ModelScope GGUF files; optional NDM_TEST_TUN_DIRECT_PORT for local listener comparison"]
async fn public_modelscope_resolve_keeps_verified_ranges() {
    let (mode, exit) = match std::env::var("NDM_TEST_TUN_DIRECT_PORT") {
        Ok(port) => (
            ProxyMode::TunDirect,
            Some(ndm_core::TunEgress::parse(&port).unwrap()),
        ),
        Err(_) => (ProxyMode::Inherit, None),
    };
    for (file, threads, hash) in [
        (
            "Q2_K",
            2,
            "8fcd5410874d03ffa430464a8bffdcbf6f53d4ea076544e7850ac3baaa1edc41",
        ),
        (
            "Q4_0",
            2,
            "5ada59352f1e9e899d3cfc860874820bfa51aa5de1ccea346eae5c2bce017815",
        ),
        (
            "Q4_0",
            16,
            "5ada59352f1e9e899d3cfc860874820bfa51aa5de1ccea346eae5c2bce017815",
        ),
    ] {
        let dir = TempDir::new().unwrap();
        let store = Store::open(&dir.path().join("tasks.db")).unwrap();
        let id = TaskId::new_v4();
        let req = request(
            dir.path(),
            format!(
                "https://www.modelscope.cn/models/second-state/SmolLM-360M-Instruct-GGUF/resolve/master/SmolLM-360M-Instruct-{file}.gguf"
            ),
        );
        store.prepare(id, &req).unwrap();
        store.commit(id).unwrap();
        let progress = DownloadProgress::default();
        let options = HttpOptions {
            max_connections: threads,
            proxy_mode: mode,
            tun_egress: exit.clone(),
            ..Default::default()
        };
        let transfer = execute_http(
            &store,
            id,
            &options,
            CancellationToken::new(),
            progress.clone(),
        );
        tokio::pin!(transfer);
        let mut peak = 0;
        let path=tokio::time::timeout(Duration::from_secs(240),async {
                loop {
                    tokio::select!{
                        result=&mut transfer=>break result,
                        _=tokio::time::sleep(Duration::from_millis(20))=>{peak=peak.max(receiving_ranges(&progress));}
                    }
                }
            }).await.unwrap().unwrap();
        let snap = progress.snapshot();
        assert_eq!(
            snap.http.as_ref().unwrap().range_mode,
            Some(ndm_core::HttpRangeMode::VerifiedSha256),
            "{:?}",
            snap.http
        );
        assert!(snap.segments.len() >= threads);
        let mut digest = Sha256::new();
        std::io::copy(&mut std::fs::File::open(path).unwrap(), &mut digest).unwrap();
        assert_eq!(hex::encode(digest.finalize()), hash);
        println!(
            "{mode:?} {file} {threads} threads: {peak} concurrently receiving, {} verified segments",
            snap.segments.len()
        );
    }
}

#[tokio::test]
#[ignore = "samples live ranges from the public 7B INT8 model, then pauses"]
async fn public_int8_model_transfers_multiple_ranges_before_pause() {
    for (site, url) in [
        (
            "ModelScope",
            "https://www.modelscope.cn/api/v1/models/Comfy-Org/SeedVR2/repo?Revision=master&FilePath=diffusion_models%2Fseedvr2_7b_int8_convrot.safetensors",
        ),
        (
            "Hugging Face",
            "https://huggingface.co/Comfy-Org/SeedVR2/resolve/main/diffusion_models/seedvr2_7b_int8_convrot.safetensors",
        ),
    ] {
        let dir = TempDir::new().unwrap();
        let mut req = request(dir.path(), url.into());
        req.suggested_filename = None;
        let id = TaskId::new_v4();
        let store = Store::open(&dir.path().join("tasks.db")).unwrap();
        store.prepare(id, &req).unwrap();
        store.commit(id).unwrap();
        let progress = DownloadProgress::default();
        let cancel = CancellationToken::new();
        let options = HttpOptions {
            max_connections: 8,
            max_bytes_per_second: Some(8 * 1024 * 1024),
            ..Default::default()
        };
        let run = execute_http(&store, id, &options, cancel.clone(), progress.clone());
        tokio::pin!(run);
        let mut seen = 0;
        let result = tokio::time::timeout(Duration::from_secs(90), async {
            loop {
                tokio::select! {
                    result = &mut run => break result,
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {
                        seen = seen.max(receiving_ranges(&progress));
                        if seen >= 2 {
                            cancel.cancel();
                        }
                    }
                }
            }
        })
        .await
        .unwrap();
        assert!(matches!(result, Err(Error::Cancelled)) && seen >= 2);
        let task = store.get(id).unwrap().unwrap();
        assert_eq!(task.state, TaskState::Paused);
        assert_eq!(
            task.engine_title.as_deref(),
            Some("seedvr2_7b_int8_convrot.safetensors")
        );
        assert_eq!(progress.snapshot().total, Some(8_334_897_976));
        assert_eq!(
            progress.snapshot().http.unwrap().range_mode,
            Some(ndm_core::HttpRangeMode::VerifiedSha256)
        );
        println!(
            "{site} 7B INT8: {seen} active ranges with received bytes; paused before completion"
        );
    }
}

#[tokio::test]
#[ignore = "downloads the public 478 MiB SeedVR2 VAE from each repository"]
async fn public_model_repositories_use_verified_parallel_ranges() {
    const HASH: &str = "20678548f420d98d26f11442d3528f8b8c94e57ee046ef93dbb7633da8612ca1";
    for (site, url) in [
        (
            "ModelScope",
            "https://www.modelscope.cn/api/v1/models/Comfy-Org/SeedVR2/repo?Revision=master&FilePath=vae%2Fseedvr2_ema_vae_fp16.safetensors",
        ),
        (
            "Hugging Face",
            "https://huggingface.co/Comfy-Org/SeedVR2/resolve/main/vae/seedvr2_ema_vae_fp16.safetensors",
        ),
    ] {
        let dir = TempDir::new().unwrap();
        let mut req = request(dir.path(), url.to_owned());
        req.suggested_filename = None;
        let id = TaskId::new_v4();
        let store = Store::open(&dir.path().join("tasks.db")).unwrap();
        store.prepare(id, &req).unwrap();
        store.commit(id).unwrap();
        let progress = DownloadProgress::default();
        let options = HttpOptions {
            max_connections: 8,
            ..Default::default()
        };
        let run = execute_http(
            &store,
            id,
            &options,
            CancellationToken::new(),
            progress.clone(),
        );
        tokio::pin!(run);
        let mut peak = 0;
        let output = tokio::time::timeout(Duration::from_secs(240), async {
            loop {
                tokio::select! {
                    result = &mut run => break result,
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {
                        peak = peak.max(receiving_ranges(&progress));
                    }
                }
            }
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            output.file_name().unwrap(),
            "seedvr2_ema_vae_fp16.safetensors"
        );
        let mut digest = Sha256::new();
        let bytes = std::io::copy(&mut std::fs::File::open(output).unwrap(), &mut digest).unwrap();
        assert_eq!(hex::encode(digest.finalize()), HASH);
        assert_eq!(
            progress.snapshot().http.unwrap().range_mode,
            Some(ndm_core::HttpRangeMode::VerifiedSha256)
        );
        assert!(peak > 1, "{site} used {peak} active ranges");
        println!("{site}: {bytes} bytes, {peak} active ranges, SHA-256 verified");
    }
}

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
    ShortOnce,
    OversizeOnce,
    WaitForThirdRange,
    LimitParallelRanges,
}

#[derive(Clone)]
struct Fixture {
    current: Arc<RwLock<(String, Vec<u8>)>>,
    mode: Mode,
    range_requests: Arc<AtomicUsize>,
    ranges: Arc<Mutex<Vec<String>>>,
    sent_bytes: Arc<AtomicUsize>,
    gate: Arc<tokio::sync::Notify>,
    rejections: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    dropped_range: Arc<Mutex<Option<String>>>,
    slow: bool,
}

async fn respond(State(fixture): State<Fixture>, headers: HeaderMap) -> Response<Body> {
    let (etag, data) = fixture.current.read().await.clone();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let number = if let Some(range) = range {
        let previous = fixture.range_requests.fetch_add(1, Ordering::SeqCst);
        fixture.ranges.lock().unwrap().push(range.to_owned());
        previous + 1
    } else {
        0
    };
    let if_range_matches = headers
        .get(header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v == etag);
    let usable_range = matches!(
        fixture.mode,
        Mode::Ranges
            | Mode::MalformedRange
            | Mode::MalformedSegment
            | Mode::DropOnce
            | Mode::OversizeOnce
            | Mode::ShortOnce
            | Mode::WaitForThirdRange
            | Mode::LimitParallelRanges
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
    if matches!(fixture.mode, Mode::OversizeOnce) && range.is_none() {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::empty())
            .unwrap();
    }
    if usable_range && range != Some("bytes=0-0") {
        match fixture.mode {
            Mode::OversizeOnce if !fixture.dropped.swap(true, Ordering::SeqCst) => {
                use futures_util::StreamExt;
                let body = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from(
                    vec![0x7b; selected.len()],
                ))])
                .chain(futures_util::stream::once(async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Ok(Bytes::from_static(b"!"))
                }));
                return response.body(Body::from_stream(body)).unwrap();
            }
            Mode::ShortOnce if !fixture.dropped.swap(true, Ordering::SeqCst) => {
                *fixture.dropped_range.lock().unwrap() = range.map(str::to_owned);
                return response
                    .body(Body::from(selected[..8192].to_vec()))
                    .unwrap();
            }
            Mode::WaitForThirdRange | Mode::LimitParallelRanges if start == 0 => {
                fixture.gate.notified().await;
            }
            Mode::WaitForThirdRange if number == 4 => fixture.gate.notify_one(),
            Mode::LimitParallelRanges => {
                let rejections = fixture.rejections.fetch_add(1, Ordering::SeqCst);
                if rejections < 3 {
                    if rejections == 2 {
                        fixture.gate.notify_one();
                    }
                    return Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .body(Body::empty())
                        .unwrap();
                }
            }
            _ => {}
        }
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
        gate: Arc::new(tokio::sync::Notify::new()),
        rejections: Arc::new(AtomicUsize::new(0)),
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

fn seed_range_manifest(
    directory: &std::path::Path,
    id: TaskId,
    url: &str,
    total: usize,
    parts: usize,
) -> std::path::PathBuf {
    let cache = directory.join(format!(".ndm-next-{id}"));
    std::fs::create_dir(&cache).unwrap();
    let manifest = serde_json::json!({
        "resource_url": url, "validator": "\"version-1\"",
        "total": total, "connections": parts
    });
    std::fs::write(
        cache.join("resume.bin"),
        ndm_core::protect_user_data(&serde_json::to_vec(&manifest).unwrap()).unwrap(),
    )
    .unwrap();
    cache
}

fn data(seed: usize) -> Vec<u8> {
    (0..524_288)
        .map(|i| ((i * 17 + seed) % 251) as u8)
        .collect()
}

#[tokio::test]
async fn a_download_without_a_user_agent_uses_chrome_for_every_range() {
    let dir = TempDir::new().unwrap();
    let expected = data(9);
    let (server, url) = fixture(Mode::Ranges, expected.clone(), false).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/file",
        get(move |headers: HeaderMap| {
            let url = url.clone();
            async move {
                let response = if headers
                    .get(header::USER_AGENT)
                    .is_some_and(|ua| ua == ndm_core::CHROME_USER_AGENT)
                {
                    Response::builder()
                        .status(StatusCode::FOUND)
                        .header(header::LOCATION, url)
                } else {
                    Response::builder().status(StatusCode::FORBIDDEN)
                };
                response.body(Body::empty()).unwrap()
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let output = download_http(
        &request(dir.path(), format!("http://{address}/file")),
        TaskId::new_v4(),
        &HttpOptions {
            max_connections: 2,
            ..Default::default()
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
    assert_eq!(server.range_requests.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_free_worker_picks_up_more_ranges_while_the_first_is_blocked() {
    let dir = TempDir::new().unwrap();
    let expected = data(22).repeat(16);
    let (_, url) = fixture(Mode::WaitForThirdRange, expected.clone(), false).await;
    // A saved eight-part layout keeps this concurrency test independent of the
    // machine's probe latency and exercises manifests created by older versions.
    let id = TaskId::new_v4();
    seed_range_manifest(dir.path(), id, &url, expected.len(), 8);
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        download_http(
            &request(dir.path(), url),
            id,
            &HttpOptions {
                max_connections: 2,
                ..Default::default()
            },
            CancellationToken::new(),
        ),
    )
    .await
    .expect("the first range waits for a third data request")
    .unwrap();
    assert_eq!(
        Sha256::digest(std::fs::read(output).unwrap()),
        Sha256::digest(expected)
    );
}

#[tokio::test]
async fn a_throttled_parallel_range_finishes_after_the_healthy_sibling() {
    let dir = TempDir::new().unwrap();
    let expected = data(23);
    let (server, url) = fixture(Mode::LimitParallelRanges, expected.clone(), false).await;
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        download_http(
            &request(dir.path(), url),
            TaskId::new_v4(),
            &HttpOptions {
                max_connections: 2,
                ..Default::default()
            },
            CancellationToken::new(),
        ),
    )
    .await
    .expect("rate limiting must not cancel the healthy sibling")
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
    assert_eq!(server.rejections.load(Ordering::SeqCst), 4);
    let ranges = server.ranges.lock().unwrap();
    assert_eq!(
        ranges
            .iter()
            .filter(|range| *range == "bytes=0-262143")
            .count(),
        1
    );
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
async fn resume_rewrites_an_oversized_part_from_zero_and_keeps_complete_parts() {
    let dir = TempDir::new().unwrap();
    let expected = data(25);
    let half = expected.len() / 2;
    let (server, url) = fixture(Mode::Ranges, expected.clone(), false).await;
    let id = TaskId::new_v4();
    let cache = seed_range_manifest(dir.path(), id, &url, expected.len(), 2);
    std::fs::write(cache.join("segment-0.part"), vec![0xff; half + 1]).unwrap();
    std::fs::write(cache.join("segment-1.part"), &expected[half..]).unwrap();

    let output = download_http(
        &request(dir.path(), url),
        id,
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
    assert_eq!(
        *server.ranges.lock().unwrap(),
        ["bytes=0-0".to_owned(), format!("bytes=0-{}", half - 1)]
    );
}

#[tokio::test]
async fn oversized_body_rolls_back_only_untrusted_bytes_before_a_later_resume() {
    let dir = TempDir::new().unwrap();
    let expected = data(24);
    let (server, url) = fixture(Mode::OversizeOnce, expected.clone(), false).await;
    let id = TaskId::new_v4();
    let cache = seed_range_manifest(dir.path(), id, &url, expected.len(), 1);
    let prefix = &expected[..8192];
    let part = cache.join("segment-0.part");
    std::fs::write(&part, prefix).unwrap();
    let req = request(dir.path(), url);
    let result = download_http(&req, id, &HttpOptions::default(), CancellationToken::new()).await;
    assert!(matches!(result, Err(Error::HttpStatus(403))), "{result:?}");
    assert_eq!(std::fs::read(&part).unwrap(), prefix);
    let output = download_http(&req, id, &HttpOptions::default(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), expected);
    assert_eq!(
        server
            .ranges
            .lock()
            .unwrap()
            .iter()
            .filter(|r| *r == "bytes=8192-524287")
            .count(),
        2
    );
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
    let partial_bytes = || {
        std::fs::read_dir(&part_dir)
            .into_iter()
            .flatten()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("segment-"))
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .sum::<u64>()
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            assert!(
                !pending.is_finished(),
                "download stopped before writing a partial response"
            );
            // The server can send a chunk before the client writes it to disk.
            if partial_bytes() > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(pending.await.unwrap().is_err());
    assert!(
        partial_bytes() > 0,
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
async fn paused_download_keeps_its_layout_after_connection_settings_change() {
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

    let fewer_connections = HttpOptions {
        max_connections: 2,
        ..Default::default()
    };
    let output = download_http(&req, id, &fewer_connections, CancellationToken::new())
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
async fn cross_origin_redirect_keeps_segments_and_user_agent_without_forwarding_private_headers() {
    let expected = Bytes::from(data(19));
    let captured = Arc::new(Mutex::new(Vec::<HeaderMap>::new()));
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination_address = destination.local_addr().unwrap();
    let receiver = Router::new().route(
        "/file",
        get({
            let captured = captured.clone();
            let expected = expected.clone();
            move |headers: HeaderMap| {
                let captured = captured.clone();
                let expected = expected.clone();
                async move {
                    let range = headers[header::RANGE].to_str().unwrap();
                    let (start, end) = range
                        .strip_prefix("bytes=")
                        .unwrap()
                        .split_once('-')
                        .unwrap();
                    let (start, end) = (
                        start.parse::<usize>().unwrap(),
                        end.parse::<usize>().unwrap(),
                    );
                    captured.lock().unwrap().push(headers);
                    Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::ETAG, "\"fixture-model\"")
                        .header(
                            header::CONTENT_RANGE,
                            format!("bytes {start}-{end}/{}", expected.len()),
                        )
                        .body(Body::from(expected.slice(start..=end)))
                        .unwrap()
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
    req.headers
        .insert("User-Agent".into(), "NDM test task".into());
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    store.prepare(id, &req).unwrap();
    store.commit(id).unwrap();
    let output = execute_http(
        &store,
        id,
        &HttpOptions {
            max_connections: 4,
            proxy_mode: ProxyMode::Direct,
            user_agent: Some("NDM test default".into()),
            ..Default::default()
        },
        CancellationToken::new(),
        DownloadProgress::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        Sha256::digest(std::fs::read(output).unwrap()),
        Sha256::digest(&expected)
    );
    let task = store.get(id).unwrap().unwrap();
    assert_eq!(task.size_verified, Some(true));
    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 5);
    for headers in captured.iter() {
        assert_eq!(headers[header::USER_AGENT], "NDM test task");
        if headers[header::RANGE] != "bytes=0-0" {
            assert_eq!(headers[header::IF_RANGE], "\"fixture-model\"");
        }
        assert!(!headers.contains_key(header::COOKIE));
        assert!(!headers.contains_key(header::AUTHORIZATION));
        assert!(!headers.contains_key("x-api-key"));
        assert!(!headers.contains_key(header::REFERER));
    }
}

#[tokio::test]
async fn rejected_range_retries_plain_get_with_the_same_direct_route_and_default_user_agent() {
    let dir = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/repo",
        get(|headers: HeaderMap| async move {
            assert_eq!(headers[header::USER_AGENT], "NDM test default");
            if headers.contains_key(header::RANGE) {
                return Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Body::empty())
                    .unwrap();
            }
            Response::builder()
                .header(
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=model.bin",
                )
                .body(Body::from("complete model"))
                .unwrap()
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    let mut req = request(dir.path(), format!("http://{address}/repo"));
    req.suggested_filename = None;
    store.prepare(id, &req).unwrap();
    store.commit(id).unwrap();
    let progress = DownloadProgress::default();
    let output = execute_http(
        &store,
        id,
        &HttpOptions {
            proxy: Some(ProxyOptions {
                url: "http://127.0.0.1:0".into(),
                username: None,
                password: None,
            }),
            proxy_mode: ProxyMode::Direct,
            user_agent: Some("NDM test default".into()),
            ..Default::default()
        },
        CancellationToken::new(),
        progress.clone(),
    )
    .await
    .unwrap();
    assert_eq!(output.file_name().unwrap().to_str(), Some("model.bin"));
    assert_eq!(std::fs::read(output).unwrap(), b"complete model");
    let http = progress.snapshot().http.unwrap();
    assert_eq!(http.range_probe_status, Some(403));
    assert_eq!(http.http_status, Some(200));
    assert_eq!(http.effective_proxy, Some(ProxyRoute::Direct));
    assert_eq!(http.failure, None);
}

#[tokio::test]
async fn range_without_a_strong_etag_still_rejects_an_incomplete_full_response() {
    let dir = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/repo",
        get(|headers: HeaderMap| async move {
            if headers.contains_key(header::RANGE) {
                assert_eq!(headers[header::RANGE], "bytes=0-0");
                return Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_RANGE, "bytes 0-0/14")
                    .body(Body::from("c"))
                    .unwrap();
            }
            Response::new(Body::from("corrupt"))
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    store
        .prepare(id, &request(dir.path(), format!("http://{address}/repo")))
        .unwrap();
    store.commit(id).unwrap();
    let result = execute_http(
        &store,
        id,
        &HttpOptions::default(),
        CancellationToken::new(),
        DownloadProgress::default(),
    )
    .await;
    assert!(matches!(result, Err(Error::InvalidHttpResponse)));
    assert!(!dir.path().join("file.bin").exists());
    let task = store.get(id).unwrap().unwrap();
    assert_eq!(task.state, TaskState::Failed);
    let http = task.progress.unwrap().http.unwrap();
    assert_eq!(http.range_probe_status, Some(206));
    assert_eq!(http.http_status, Some(200));
    assert_eq!(http.stage, HttpStage::IntegrityCheck);
    assert_eq!(http.failure, Some(DownloadFailure::InvalidResponse));
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
async fn rotating_cdn_signatures_keep_ranges_only_for_the_same_representation() {
    use ndm_core::{HttpRangeMode, SegmentIssue};
    for (change, failure) in [
        ("none", None),
        ("etag", Some(SegmentIssue::EtagMismatch)),
        ("range", Some(SegmentIssue::ContentRangeMismatch)),
        ("total", Some(SegmentIssue::ContentRangeMismatch)),
        ("path", Some(SegmentIssue::ResourceChanged)),
        ("selector", Some(SegmentIssue::ResourceChanged)),
    ] {
        let dir = TempDir::new().unwrap();
        let expected = data(19);
        let body = Bytes::copy_from_slice(&expected);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let app = Router::new()
            .route(
                "/original",
                get(move |headers: HeaderMap| {
                    let seq = counter.fetch_add(1, Ordering::SeqCst);
                    let segment = headers.contains_key(header::IF_RANGE);
                    let path = if segment && change == "path" {
                        "other"
                    } else {
                        "model"
                    };
                    let id = if segment && change == "selector" {
                        2
                    } else {
                        1
                    };
                    async move {
                        (
                            StatusCode::FOUND,
                            [(
                                header::LOCATION,
                                format!("/cdn/{path}?id={id}&signature={seq}"),
                            )],
                        )
                    }
                }),
            )
            .route(
                "/cdn/{file}",
                get(move |headers: HeaderMap| {
                    let body = body.clone();
                    async move {
                        let Some(range) = headers.get(header::RANGE) else {
                            return Response::new(Body::from(body));
                        };
                        let segment = headers.contains_key(header::IF_RANGE);
                        if segment {
                            assert_eq!(headers[header::IF_RANGE], "\"stable\"");
                        }
                        let (start, end) = range.to_str().unwrap()[6..].split_once('-').unwrap();
                        let start: usize = start.parse().unwrap();
                        let end: usize = end.parse().unwrap();
                        let total = body.len() + usize::from(segment && change == "total");
                        let first = start + usize::from(segment && change == "range");
                        Response::builder()
                            .status(StatusCode::PARTIAL_CONTENT)
                            .header(
                                header::ETAG,
                                if segment && change == "etag" {
                                    "\"new\""
                                } else {
                                    "\"stable\""
                                },
                            )
                            .header(
                                header::CONTENT_RANGE,
                                format!("bytes {first}-{end}/{total}"),
                            )
                            .body(Body::from(body.slice(start..=end)))
                            .unwrap()
                    }
                }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let store = Store::open(&dir.path().join("tasks.db")).unwrap();
        let id = TaskId::new_v4();
        store
            .prepare(
                id,
                &request(dir.path(), format!("http://{address}/original")),
            )
            .unwrap();
        store.commit(id).unwrap();
        let progress = DownloadProgress::default();
        execute_http(
            &store,
            id,
            &HttpOptions {
                max_connections: 16,
                ..Default::default()
            },
            CancellationToken::new(),
            progress.clone(),
        )
        .await
        .unwrap();
        let snapshot = progress.snapshot();
        let http = snapshot.http.unwrap();
        assert_eq!(http.segment_failure.map(|f| f.issue), failure, "{change}");
        if failure.is_none() {
            assert!(snapshot.segments.len() > 1);
            assert!(calls.load(Ordering::SeqCst) > 2);
            assert_eq!(http.range_mode, Some(HttpRangeMode::StrongEtag));
        } else {
            assert_eq!(http.range_mode, Some(HttpRangeMode::SegmentFallback));
        }
        assert_eq!(
            std::fs::read(dir.path().join("file.bin")).unwrap(),
            expected
        );
    }
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
async fn rate_limited_download_waits_for_retry_after() {
    let dir = TempDir::new().unwrap();
    let first_request = Arc::new(std::sync::OnceLock::new());
    let requests = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/file",
        get({
            let requests = requests.clone();
            move || {
                let first_request = first_request.clone();
                requests.fetch_add(1, Ordering::SeqCst);
                async move {
                    let elapsed = first_request.get_or_init(std::time::Instant::now).elapsed();
                    if elapsed < Duration::from_secs(1) {
                        (
                            StatusCode::TOO_MANY_REQUESTS,
                            [(header::RETRY_AFTER, "1")],
                            "",
                        )
                    } else {
                        (
                            StatusCode::OK,
                            [(header::CONTENT_TYPE, "text/plain")],
                            "recovered",
                        )
                    }
                }
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let output = download_http(
        &request(dir.path(), format!("http://{address}/file")),
        TaskId::new_v4(),
        &HttpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"recovered");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn excessive_retry_after_preserves_http_status_without_an_early_request() {
    let dir = TempDir::new().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/file",
        get({
            let requests = requests.clone();
            move || {
                requests.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [(header::RETRY_AFTER, "3600")],
                    )
                }
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        download_http(
            &request(dir.path(), format!("http://{address}/file")),
            TaskId::new_v4(),
            &HttpOptions::default(),
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(Error::HttpStatus(503))));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
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
async fn disconnected_or_short_segment_is_retried_from_its_saved_offset() {
    for mode in [Mode::DropOnce, Mode::ShortOnce] {
        let dir = TempDir::new().unwrap();
        let expected = data(15);
        let (server, url) = fixture(mode, expected.clone(), false).await;
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
