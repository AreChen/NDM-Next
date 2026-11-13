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
