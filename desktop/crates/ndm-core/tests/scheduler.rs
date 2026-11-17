use axum::{
    Router,
    body::Body,
    http::{Response, StatusCode, header},
    routing::get,
};
use ndm_core::{NewTask, Scheduler, SchedulerOptions, Store, TaskId, TaskState};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;

async fn server() -> (String, Arc<AtomicUsize>) {
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let handler = {
        let current = current.clone();
        let peak = peak.clone();
        move || {
            let current = current.clone();
            let peak = peak.clone();
            async move {
                let count = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(120)).await;
                current.fetch_sub(1, Ordering::SeqCst);
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_LENGTH, 1024)
                    .body(Body::from(vec![42; 1024]))
                    .unwrap()
            }
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/file", get(handler)))
            .await
            .unwrap();
    });
    (format!("http://{address}/file"), peak)
}

fn request(directory: &std::path::Path, url: &str) -> NewTask {
    NewTask {
        url: url.into(),
        destination: directory.to_path_buf(),
        suggested_filename: Some("file.bin".into()),
        source_url: None,
        expected_size: None,
        headers: BTreeMap::new(),
    }
}

async fn wait_for(store: &Store, id: TaskId, state: TaskState) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if store
                .get(id)
                .unwrap()
                .is_some_and(|task| task.state == state)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

async fn streaming_server(known_length: bool) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/stream",
        get(move || async move {
            let chunks = futures_util::stream::unfold(0, |index| async move {
                if index == 16 {
                    return None;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((Ok::<_, std::io::Error>(vec![42; 4096]), index + 1))
            });
            let mut response = Response::builder().status(StatusCode::OK);
            if known_length {
                response = response.header(header::CONTENT_LENGTH, 65_536);
            }
            response.body(Body::from_stream(chunks)).unwrap()
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{address}/stream")
}

#[tokio::test]
async fn custom_cache_location_survives_pause_and_applies_to_future_tasks() {
    let directory = TempDir::new().unwrap();
    let destination = if let Some(root) = std::env::var_os("NDM_TEST_OTHER_VOLUME") {
        TempDir::new_in(root).unwrap()
    } else {
        TempDir::new().unwrap()
    };
    let store = Arc::new(Store::open(&directory.path().join("tasks.db")).unwrap());
    let first_cache = directory.path().join("cache-one");
    let second_cache = directory.path().join("cache-two");
    let mut options = SchedulerOptions {
        cache_directory: Some(first_cache.clone()),
        ..Default::default()
    };
    let scheduler = Scheduler::start(store.clone(), options.clone()).unwrap();
    let url = streaming_server(true).await;
    let id = TaskId::new_v4();
    store
        .prepare(id, &request(destination.path(), &url))
        .unwrap();
    store.commit(id).unwrap();
    scheduler.notify();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if scheduler
                .progress(id)
                .is_some_and(|progress| progress.downloaded >= 8192)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    scheduler.pause(id).await.unwrap();
    let paused = store.get(id).unwrap().unwrap();
    assert!(
        paused
            .work_directory()
            .starts_with(std::fs::canonicalize(&first_cache).unwrap())
    );
    assert!(paused.work_directory().is_dir());
    assert!(!destination.path().join(format!(".ndm-next-{id}")).exists());
    options.cache_directory = Some(second_cache.clone());
    scheduler.update_options(options);
    let report = scheduler.clean_cache(None).unwrap();
    assert_eq!(report.removed_bytes, 0);
    assert!(report.protected_bytes >= 8192);
    scheduler.resume(id).await.unwrap();
    wait_for(&store, id, TaskState::Completed).await;
    let finished = store.get(id).unwrap().unwrap();
    assert_eq!(finished.cache_directory, paused.cache_directory);
    assert_eq!(
        std::fs::read(finished.output_path.unwrap()).unwrap(),
        vec![42; 65_536]
    );
    let next = TaskId::new_v4();
    store
        .prepare(next, &request(destination.path(), &url))
        .unwrap();
    store.commit(next).unwrap();
    scheduler.notify();
    wait_for(&store, next, TaskState::Completed).await;
    assert_eq!(
        store.get(next).unwrap().unwrap().cache_directory.unwrap(),
        std::fs::canonicalize(second_cache).unwrap()
    );
    scheduler.shutdown().await;
}

#[tokio::test]
async fn task_proxy_switch_preserves_request_and_statistics_without_changing_other_tasks() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let store = Arc::new(Store::open(&database).unwrap());
    let url = streaming_server(true).await;
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = ndm_core::ProxyOptions {
        url: format!("http://{}", listener.local_addr().unwrap()),
        username: Some("task-user".into()),
        password: Some("task-secret".into()),
    };
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(move |headers: axum::http::HeaderMap| {
                let count = count.clone();
                async move {
                    assert!(headers.contains_key(header::PROXY_AUTHORIZATION));
                    count.fetch_add(1, Ordering::SeqCst);
                    let chunks = futures_util::stream::unfold(0, |index| async move {
                        if index == 16 {
                            return None;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Some((Ok::<_, std::io::Error>(vec![42; 4096]), index + 1))
                    });
                    Response::builder()
                        .header(header::CONTENT_LENGTH, 65_536)
                        .body(Body::from_stream(chunks))
                        .unwrap()
                }
            }),
        )
        .await
        .unwrap();
    });
    let id = TaskId::new_v4();
    let mut original = request(dir.path(), &url);
    original
        .headers
        .insert("Cookie".into(), "media-session=preserved".into());
    store.prepare(id, &original).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while scheduler.progress(id).is_none_or(|p| p.downloaded == 0) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    scheduler.retry_with_proxy(id, proxy.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while hits.load(Ordering::SeqCst) == 0
            || scheduler.progress(id).is_none_or(|p| p.downloaded == 0)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    scheduler.pause(id).await.unwrap();
    let paused = store.get(id).unwrap().unwrap();
    assert_eq!(paused.request, original);
    assert!(
        matches!(&paused.options, ndm_core::TaskOptions::Http(options) if options.proxy.as_ref() == Some(&proxy))
    );
    assert!(paused.progress.as_ref().unwrap().transferred_total > 0);
    scheduler.shutdown().await;
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    let before_resume = hits.load(Ordering::SeqCst);
    scheduler.resume(id).await.unwrap();
    wait_for(&store, id, TaskState::Completed).await;
    let completed = store.get(id).unwrap().unwrap();
    assert!(hits.load(Ordering::SeqCst) > before_resume);
    assert_eq!(
        std::fs::read(completed.output_path.unwrap()).unwrap(),
        vec![42; 65_536]
    );
    assert!(
        completed.progress.unwrap().transferred_total > paused.progress.unwrap().transferred_total
    );
    let before_other = hits.load(Ordering::SeqCst);
    let other = TaskId::new_v4();
    store.prepare(other, &request(dir.path(), &url)).unwrap();
    store.commit(other).unwrap();
    scheduler.notify();
    wait_for(&store, other, TaskState::Completed).await;
    assert_eq!(hits.load(Ordering::SeqCst), before_other);
    assert_eq!(
        store.get(other).unwrap().unwrap().options,
        ndm_core::TaskOptions::Auto
    );
    scheduler.shutdown().await;
}

#[tokio::test]
async fn proxy_retry_is_atomic_when_an_installer_has_reserved_the_idle_profile() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let id = TaskId::new_v4();
    store
        .prepare(id, &request(dir.path(), "https://example.test/video.m3u8"))
        .unwrap();
    store.commit(id).unwrap();
    store.pause(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    let reservation = store.reserve_update_if_idle().unwrap().unwrap();
    let proxy = ndm_core::ProxyOptions {
        url: "http://127.0.0.1:18085".into(),
        username: None,
        password: None,
    };
    assert!(scheduler.retry_with_proxy(id, proxy).await.is_err());
    let task = store.get(id).unwrap().unwrap();
    assert_eq!(task.state, TaskState::Paused);
    assert_eq!(task.options, ndm_core::TaskOptions::Auto);
    drop(reservation);
    scheduler.shutdown().await;
}

#[tokio::test]
async fn saved_progress_survives_pause_reopen_and_resume_without_counting_pause_time() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let store = Arc::new(Store::open(&database).unwrap());
    let url = streaming_server(true).await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while scheduler
            .progress(id)
            .is_none_or(|progress| progress.downloaded == 0)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    scheduler.pause(id).await.unwrap();
    let paused = store.get(id).unwrap().unwrap().progress.unwrap();
    assert!(paused.downloaded > 0);
    assert_eq!(paused.total, Some(65_536));
    assert!(paused.elapsed_ms > 0);
    assert!(paused.transferred_total > 0);
    assert!(paused.segments.iter().all(|segment| !segment.active));
    scheduler.shutdown().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let reopened = Arc::new(Store::open(&database).unwrap());
    assert_eq!(
        reopened.get(id).unwrap().unwrap().progress.as_ref(),
        Some(&paused)
    );
    let scheduler = Scheduler::start(reopened.clone(), SchedulerOptions::default()).unwrap();
    scheduler.resume(id).await.unwrap();
    wait_for(&reopened, id, TaskState::Completed).await;
    scheduler.shutdown().await;
    let completed = reopened.get(id).unwrap().unwrap().progress.unwrap();
    assert_eq!(completed.downloaded, 65_536);
    assert_eq!(completed.total, Some(65_536));
    assert!(completed.elapsed_ms >= paused.elapsed_ms);
    assert_eq!(
        completed.transferred_total,
        paused.transferred_total + 65_536
    );
    assert!(
        completed
            .average_bytes_per_second()
            .is_some_and(|speed| speed > 0)
    );
}

#[tokio::test]
async fn saved_progress_records_actual_unknown_length_without_claiming_length_verification() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let url = streaming_server(false).await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    wait_for(&store, id, TaskState::Completed).await;
    scheduler.shutdown().await;
    let task = store.get(id).unwrap().unwrap();
    assert_eq!(task.size_verified, Some(false));
    let progress = task.progress.unwrap();
    assert_eq!(progress.downloaded, 65_536);
    assert_eq!(progress.total, Some(65_536));
    assert_eq!(progress.transferred_total, 65_536);
    assert_eq!(
        std::fs::metadata(task.output_path.unwrap()).unwrap().len(),
        65_536
    );
}

#[tokio::test]
async fn failed_resume_keeps_owned_partial_bytes_in_the_saved_snapshot() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let id = TaskId::new_v4();
    let url = streaming_server(true).await;
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while scheduler
            .progress(id)
            .is_none_or(|progress| progress.downloaded == 0)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    scheduler.pause(id).await.unwrap();
    let saved = store.get(id).unwrap().unwrap().progress.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = format!("http://{}/file", listener.local_addr().unwrap());
    drop(listener);
    store.update_web_url(id, &unavailable).unwrap();
    scheduler.resume(id).await.unwrap();
    wait_for(&store, id, TaskState::Failed).await;
    scheduler.shutdown().await;
    let failed = store.get(id).unwrap().unwrap().progress.unwrap();
    assert_eq!(failed.downloaded, saved.downloaded);
    assert!(failed.downloaded > 0);
    assert_eq!(failed.transferred_total, saved.transferred_total);
    assert!(failed.elapsed_ms >= saved.elapsed_ms);
}

#[tokio::test]
async fn paused_average_includes_received_bytes_waiting_for_the_rate_limiter() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let id = TaskId::new_v4();
    let url = streaming_server(true).await;
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let mut options = SchedulerOptions::default();
    options.http.max_bytes_per_second = Some(1);
    let scheduler = Scheduler::start(store.clone(), options).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while scheduler
            .progress(id)
            .is_none_or(|progress| progress.transferred <= progress.downloaded)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    scheduler.pause(id).await.unwrap();
    let paused = store.get(id).unwrap().unwrap().progress.unwrap();
    assert!(paused.transferred_total > paused.downloaded);
    assert_eq!(paused.downloaded, 0);
    scheduler.shutdown().await;
}

#[tokio::test]
async fn scheduler_runs_queued_tasks_with_bounded_concurrency() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, peak) = server().await;
    let ids = [TaskId::new_v4(), TaskId::new_v4()];
    for id in ids {
        store.prepare(id, &request(dir.path(), &url)).unwrap();
        store.commit(id).unwrap();
    }
    let scheduler = Scheduler::start(
        store.clone(),
        SchedulerOptions {
            max_active_tasks: 1,
            ..Default::default()
        },
    )
    .unwrap();
    for id in ids {
        wait_for(&store, id, TaskState::Completed).await;
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read(dir.path().join("file.bin")).unwrap(),
        vec![42; 1024]
    );
    assert_eq!(
        std::fs::read(dir.path().join("file (1).bin")).unwrap(),
        vec![42; 1024]
    );
    scheduler.shutdown().await;
}

#[tokio::test]
async fn updating_concurrency_wakes_waiting_tasks() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, peak) = server().await;
    let ids = [TaskId::new_v4(), TaskId::new_v4()];
    store.prepare(ids[0], &request(dir.path(), &url)).unwrap();
    store.commit(ids[0]).unwrap();
    let scheduler = Scheduler::start(
        store.clone(),
        SchedulerOptions {
            max_active_tasks: 1,
            ..Default::default()
        },
    )
    .unwrap();
    wait_for(&store, ids[0], TaskState::Running).await;
    store.prepare(ids[1], &request(dir.path(), &url)).unwrap();
    store.commit(ids[1]).unwrap();
    scheduler.update_options(SchedulerOptions {
        max_active_tasks: 2,
        ..Default::default()
    });
    for id in ids {
        wait_for(&store, id, TaskState::Completed).await;
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    scheduler.shutdown().await;
}

#[tokio::test]
async fn pausing_a_running_task_remains_paused_until_resumed() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, _) = server().await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    wait_for(&store, id, TaskState::Running).await;
    scheduler.pause(id).await.unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Paused);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Paused);
    scheduler.resume(id).await.unwrap();
    wait_for(&store, id, TaskState::Completed).await;
    scheduler.shutdown().await;
}

#[tokio::test]
async fn deleting_a_completed_record_can_remove_leftover_owned_cache() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, _) = server().await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    wait_for(&store, id, TaskState::Completed).await;
    let task = store.get(id).unwrap().unwrap();
    let cache = task.work_directory();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("assembled.part"), b"leftover cache").unwrap();
    scheduler.delete(id, true).unwrap();
    assert!(store.get(id).unwrap().is_none());
    assert!(!cache.exists());
    // The scheduler only owns temporary data; the desktop file layer handles
    // deleting/recycling the published output before deleting the record.
    assert!(task.output_path.unwrap().exists());
    scheduler.shutdown().await;
}

#[tokio::test]
async fn removing_a_paused_task_can_keep_or_remove_only_its_temporary_files() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let mut tasks = Vec::new();
    for remove_partial in [false, true] {
        let id = TaskId::new_v4();
        store
            .prepare(id, &request(dir.path(), "https://example.test/file"))
            .unwrap();
        store.commit(id).unwrap();
        store.pause(id).unwrap();
        let work_dir = dir.path().join(format!(".ndm-next-{id}"));
        std::fs::create_dir(&work_dir).unwrap();
        std::fs::write(work_dir.join("segment-0.part"), b"partial").unwrap();
        std::fs::write(
            work_dir.join("hls-output.mp4.hls-partial"),
            b"partial media",
        )
        .unwrap();
        std::fs::write(work_dir.join("hls-checkpoint.sqlite"), b"checkpoint").unwrap();
        let completed_file = dir.path().join(format!("completed-{id}.bin"));
        std::fs::write(&completed_file, b"keep me").unwrap();
        tasks.push((id, remove_partial, work_dir, completed_file));
    }
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    for (id, remove_partial, work_dir, completed_file) in tasks {
        scheduler.delete(id, remove_partial).unwrap();
        assert!(store.get(id).unwrap().is_none());
        assert_eq!(work_dir.exists(), !remove_partial);
        assert_eq!(std::fs::read(completed_file).unwrap(), b"keep me");
    }
    scheduler.shutdown().await;
}

#[tokio::test]
async fn temporary_cleanup_refuses_unexpected_files_and_retains_the_record() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let id = TaskId::new_v4();
    store
        .prepare(id, &request(dir.path(), "https://example.test/file"))
        .unwrap();
    store.commit(id).unwrap();
    store.pause(id).unwrap();
    let work_dir = dir.path().join(format!(".ndm-next-{id}"));
    std::fs::create_dir(&work_dir).unwrap();
    std::fs::write(work_dir.join("important.txt"), b"keep me").unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    assert!(scheduler.delete(id, true).is_err());
    assert!(store.get(id).unwrap().is_some());
    assert_eq!(
        std::fs::read(work_dir.join("important.txt")).unwrap(),
        b"keep me"
    );
    scheduler.shutdown().await;
}

#[tokio::test]
async fn removing_an_active_task_waits_for_the_worker_to_stop() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, _) = server().await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    wait_for(&store, id, TaskState::Running).await;
    assert!(scheduler.delete(id, true).is_err());
    scheduler.pause(id).await.unwrap();
    scheduler.delete(id, true).unwrap();
    assert!(store.get(id).unwrap().is_none());
    scheduler.shutdown().await;
}

#[tokio::test]
async fn external_discard_stops_an_active_copy_and_never_deletes_completed_files() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, _) = server().await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    wait_for(&store, id, TaskState::Running).await;
    store.request_discard(id).unwrap();
    scheduler.notify();
    tokio::time::timeout(Duration::from_secs(5), async {
        while store.get(id).unwrap().is_some() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(!dir.path().join("file.bin").exists());
    let finished = TaskId::new_v4();
    store.prepare(finished, &request(dir.path(), &url)).unwrap();
    store.commit(finished).unwrap();
    scheduler.notify();
    wait_for(&store, finished, TaskState::Completed).await;
    store.request_discard(finished).unwrap();
    scheduler.notify();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        store.get(finished).unwrap().unwrap().state,
        TaskState::Completed
    );
    assert_eq!(
        std::fs::read(dir.path().join("file.bin")).unwrap(),
        vec![42; 1024]
    );
    scheduler.shutdown().await;
}

#[tokio::test]
async fn already_paused_tasks_do_not_restart_on_startup() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, _) = server().await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    store.pause(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Paused);
    scheduler.shutdown().await;
}

#[tokio::test]
async fn shutdown_preserves_unfinished_work_for_next_start() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, _) = server().await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    wait_for(&store, id, TaskState::Running).await;
    scheduler.shutdown().await;
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Queued);
    let reopened = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    wait_for(&store, id, TaskState::Completed).await;
    reopened.shutdown().await;
}

#[tokio::test]
async fn disabling_auto_resume_keeps_interrupted_jobs_paused() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let (url, _) = server().await;
    let id = TaskId::new_v4();
    store.prepare(id, &request(dir.path(), &url)).unwrap();
    store.commit(id).unwrap();
    store.start(id).unwrap();
    let scheduler = Scheduler::start(
        store.clone(),
        SchedulerOptions {
            auto_resume: false,
            ..Default::default()
        },
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Paused);
    scheduler.shutdown().await;
}
