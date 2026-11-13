use std::{collections::BTreeMap, fs, path::Path};

use ndm_core::{NewTask, Store, TaskId, TaskState};
use tempfile::TempDir;

fn new_task(destination: &Path) -> NewTask {
    NewTask {
        url: "https://example.test/file.zip?token=secret-download-token".into(),
        destination: destination.to_path_buf(),
        suggested_filename: Some("file.zip".into()),
        source_url: Some("https://example.test/page".into()),
        expected_size: Some(1024),
        headers: BTreeMap::from([("Cookie".into(), "session=secret-cookie".into())]),
    }
}

#[test]
fn prepare_and_commit_are_durable_and_idempotent() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let id = TaskId::new_v4();
    let request = new_task(dir.path());

    let store = Store::open(&database).unwrap();
    assert_eq!(
        store.prepare(id, &request).unwrap().state,
        TaskState::Prepared
    );
    assert_eq!(store.prepare(id, &request).unwrap().id, id);
    assert_eq!(store.commit(id).unwrap().state, TaskState::Queued);
    assert_eq!(store.commit(id).unwrap().state, TaskState::Queued);
    drop(store);

    let reopened = Store::open(&database).unwrap();
    let task = reopened.get(id).unwrap().unwrap();
    assert_eq!(task.request, request);
    assert_eq!(task.state, TaskState::Queued);
    assert_eq!(reopened.list().unwrap().len(), 1);
}

#[test]
fn reused_identifier_with_different_request_is_rejected() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    let request = new_task(dir.path());
    store.prepare(id, &request).unwrap();

    let mut other = request.clone();
    other.url = "https://example.test/other.zip".into();
    assert!(store.prepare(id, &other).is_err());
    assert_eq!(store.get(id).unwrap().unwrap().request, request);
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn aborted_preparation_cannot_be_committed() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    store.prepare(id, &new_task(dir.path())).unwrap();
    store.abort(id).unwrap();
    assert!(store.get(id).unwrap().is_none());
    assert!(store.commit(id).is_err());
}

#[test]
fn recovery_requeues_interrupted_work_but_preserves_manual_pause() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let active = TaskId::new_v4();
    let paused = TaskId::new_v4();
    let store = Store::open(&database).unwrap();
    for id in [active, paused] {
        store.prepare(id, &new_task(dir.path())).unwrap();
        store.commit(id).unwrap();
        store.start(id).unwrap();
    }
    store.pause(paused).unwrap();
    drop(store);

    let reopened = Store::open(&database).unwrap();
    reopened.recover_interrupted().unwrap();
    assert_eq!(
        reopened.get(active).unwrap().unwrap().state,
        TaskState::Queued
    );
    assert_eq!(
        reopened.get(paused).unwrap().unwrap().state,
        TaskState::Paused
    );
}

#[test]
fn request_secrets_are_not_stored_as_plaintext() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let id = TaskId::new_v4();
    let request = new_task(dir.path());
    let store = Store::open(&database).unwrap();
    store.prepare(id, &request).unwrap();
    drop(store);

    for entry in fs::read_dir(dir.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let contents = fs::read(path).unwrap();
            assert!(
                !contents
                    .windows(b"secret-cookie".len())
                    .any(|w| w == b"secret-cookie")
            );
            assert!(
                !contents
                    .windows(b"secret-download-token".len())
                    .any(|w| w == b"secret-download-token")
            );
        }
    }
    assert_eq!(
        Store::open(&database)
            .unwrap()
            .get(id)
            .unwrap()
            .unwrap()
            .request,
        request
    );
}

#[test]
fn older_task_database_is_upgraded_without_losing_the_table() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE tasks (
        id TEXT PRIMARY KEY NOT NULL, request BLOB NOT NULL, state TEXT NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );",
        )
        .unwrap();
    drop(connection);
    let store = Store::open(&database).unwrap();
    let id = TaskId::new_v4();
    store.prepare(id, &new_task(dir.path())).unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Prepared);
}

#[test]
fn concurrent_duplicate_prepares_create_only_one_task() {
    let dir = TempDir::new().unwrap();
    let store = std::sync::Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let id = TaskId::new_v4();
    let request = new_task(dir.path());
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            let request = request.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.prepare(id, &request).unwrap()
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap().id, id);
    }
    assert_eq!(store.list().unwrap().len(), 1);
}
