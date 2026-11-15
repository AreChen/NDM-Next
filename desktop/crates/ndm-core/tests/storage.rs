use ndm_core::{NewTask, Scheduler, SchedulerOptions, Store, TaskId};
use std::{collections::BTreeMap, fs, sync::Arc, time::SystemTime};

fn task(store: &Store, destination: &std::path::Path) -> TaskId {
    let id = TaskId::new_v4();
    store
        .prepare(
            id,
            &NewTask {
                url: "https://example.com/file".into(),
                destination: destination.into(),
                suggested_filename: None,
                source_url: None,
                expected_size: None,
                headers: BTreeMap::new(),
            },
        )
        .unwrap();
    store.commit(id).unwrap();
    store.pause(id).unwrap();
    id
}

#[tokio::test]
async fn expired_cleanup_preserves_all_unfinished_states_and_completed_output() {
    use ndm_core::TaskState;
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("tasks.db")).unwrap());
    let scheduler = Scheduler::start(
        store.clone(),
        SchedulerOptions {
            auto_resume: false,
            ..Default::default()
        },
    )
    .unwrap();
    let cache = directory.path().join("cache");
    let mut protected = Vec::new();
    for state in [
        TaskState::Prepared,
        TaskState::Queued,
        TaskState::Running,
        TaskState::Paused,
        TaskState::Failed,
        TaskState::Completed,
    ] {
        let id = TaskId::new_v4();
        let request = NewTask {
            url: "https://example.com/test".into(),
            destination: directory.path().join("output"),
            suggested_filename: None,
            source_url: None,
            expected_size: None,
            headers: BTreeMap::new(),
        };
        store.prepare(id, &request).unwrap();
        let saved = store.assign_workspace(id, Some(&cache)).unwrap();
        fs::create_dir_all(saved.work_directory()).unwrap();
        fs::write(saved.work_directory().join("assembled.part"), [1; 64]).unwrap();
        if state != TaskState::Prepared {
            store.commit(id).unwrap();
        }
        if matches!(
            state,
            TaskState::Running | TaskState::Failed | TaskState::Completed
        ) {
            store.start(id).unwrap();
        }
        match state {
            TaskState::Paused => {
                store.pause(id).unwrap();
            }
            TaskState::Failed => {
                rusqlite::Connection::open(directory.path().join("tasks.db"))
                    .unwrap()
                    .execute(
                        "UPDATE tasks SET state = 'failed' WHERE id = ?1",
                        [id.to_string()],
                    )
                    .unwrap();
            }
            TaskState::Completed => {
                let output = directory.path().join("completed.bin");
                fs::write(&output, b"completed file").unwrap();
                let encrypted =
                    ndm_core::protect_user_data(&serde_json::to_vec(&output).unwrap()).unwrap();
                rusqlite::Connection::open(directory.path().join("tasks.db")).unwrap().execute("UPDATE tasks SET state = 'completed', output = ?2, size_verified = 1 WHERE id = ?1", rusqlite::params![id.to_string(), encrypted]).unwrap();
            }
            _ => {}
        }
        if state != TaskState::Completed {
            protected.push(saved.work_directory());
        }
    }
    let report = scheduler
        .clean_cache(Some(SystemTime::now() + std::time::Duration::from_secs(2)))
        .unwrap();
    assert_eq!(report.removed_bytes, 64);
    assert_eq!(report.protected_bytes, 5 * 64);
    for path in protected {
        assert!(path.join("assembled.part").exists());
    }
    assert_eq!(
        fs::read(directory.path().join("completed.bin")).unwrap(),
        b"completed file"
    );
    scheduler.shutdown().await;
}

#[test]
fn database_migration_preserves_legacy_partial_files() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("tasks.db");
    let store = Store::open(&database).unwrap();
    let destination = directory.path().join("downloads");
    let id = task(&store, &destination);
    let legacy = destination.join(format!(".ndm-next-{id}"));
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("segment-0.part"), b"resume").unwrap();
    drop(store);
    let old = rusqlite::Connection::open(&database).unwrap();
    old.execute_batch("DROP TABLE cache_workspaces; PRAGMA user_version = 2;")
        .unwrap();
    drop(old);
    let migrated = Store::open(&database).unwrap();
    let saved = migrated
        .assign_workspace(id, Some(&directory.path().join("new-cache")))
        .unwrap();
    assert_eq!(saved.work_directory(), fs::canonicalize(&legacy).unwrap());
    assert_eq!(
        fs::read(saved.work_directory().join("segment-0.part")).unwrap(),
        b"resume"
    );
}

#[tokio::test]
async fn workspace_is_persisted_and_cleanup_protects_resume_data() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("tasks.db");
    let store = Arc::new(Store::open(&database).unwrap());
    let first = directory.path().join("cache-one");
    let second = directory.path().join("cache-two");
    let id = task(&store, &directory.path().join("downloads"));
    let saved = store.assign_workspace(id, Some(&first)).unwrap();
    assert_eq!(
        saved.cache_directory.as_deref(),
        Some(fs::canonicalize(&first).unwrap().as_path())
    );
    fs::create_dir_all(saved.work_directory()).unwrap();
    fs::write(saved.work_directory().join("segment-0.part"), [1; 256]).unwrap();
    assert_eq!(
        store
            .assign_workspace(id, Some(&second))
            .unwrap()
            .work_directory(),
        saved.work_directory()
    );
    assert_eq!(
        Store::open(&database)
            .unwrap()
            .get(id)
            .unwrap()
            .unwrap()
            .work_directory(),
        saved.work_directory()
    );
    let scheduler = Scheduler::start(
        store.clone(),
        SchedulerOptions {
            auto_resume: false,
            ..Default::default()
        },
    )
    .unwrap();
    let report = scheduler.clean_cache(None).unwrap();
    assert_eq!(report.removed_bytes, 0);
    assert_eq!(report.protected_bytes, 256);
    assert!(saved.work_directory().exists());
    scheduler.delete(id, false).unwrap();
    assert_eq!(
        scheduler
            .clean_cache(Some(SystemTime::UNIX_EPOCH))
            .unwrap()
            .removed_bytes,
        0
    );
    let report = scheduler.clean_cache(None).unwrap();
    assert_eq!(report.removed_bytes, 256);
    assert!(!saved.work_directory().exists());
    scheduler.shutdown().await;
}

#[tokio::test]
async fn legacy_workspaces_are_kept_and_unexpected_files_are_never_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("tasks.db")).unwrap());
    let destination = directory.path().join("downloads");
    let id = task(&store, &destination);
    let legacy = destination.join(format!(".ndm-next-{id}"));
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("resume.bin"), [1; 64]).unwrap();
    assert_eq!(
        store
            .assign_workspace(id, Some(&directory.path().join("new-cache")))
            .unwrap()
            .work_directory(),
        fs::canonicalize(&legacy).unwrap()
    );
    let scheduler = Scheduler::start(
        store,
        SchedulerOptions {
            auto_resume: false,
            ..Default::default()
        },
    )
    .unwrap();
    scheduler.delete(id, false).unwrap();
    fs::write(legacy.join("personal.txt"), b"keep me").unwrap();
    let report = scheduler.clean_cache(None).unwrap();
    assert_eq!(report.removed_bytes, 0);
    assert!(report.skipped_directories > 0);
    assert!(legacy.join("resume.bin").exists());
    assert_eq!(fs::read(legacy.join("personal.txt")).unwrap(), b"keep me");
    scheduler.shutdown().await;
}
