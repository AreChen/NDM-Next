use std::{collections::BTreeMap, fs, path::Path};

use ndm_core::{NewTask, SftpAuth, SftpOptions, Store, TaskId, TaskOptions, TaskState};
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

fn assert_update_blocked<T>(result: ndm_core::Result<T>) {
    assert!(
        matches!(result, Err(ndm_core::Error::Io(ref error)) if error.kind() == std::io::ErrorKind::WouldBlock),
        "work was admitted while an update reservation was held"
    );
}

#[test]
fn update_reservation_blocks_commit_and_resume_until_dropped() {
    fn send<T: Send>() {}
    send::<ndm_core::DesktopUpdateGuard>();
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let store = Store::open(&database).unwrap();
    let prepared = TaskId::new_v4();
    let paused = TaskId::new_v4();
    for id in [prepared, paused] {
        store.prepare(id, &new_task(dir.path())).unwrap();
    }
    store.commit(paused).unwrap();
    store.pause(paused).unwrap();
    let reservation = store.reserve_update_if_idle().unwrap().unwrap();
    assert!(ndm_core::desktop_installation_active(dir.path()).unwrap());
    let other = Store::open(&fs::canonicalize(&database).unwrap()).unwrap();
    assert_update_blocked(other.commit(prepared));
    assert_update_blocked(other.resume(paused));
    assert_eq!(
        store.get(prepared).unwrap().unwrap().state,
        TaskState::Prepared
    );
    assert_eq!(store.get(paused).unwrap().unwrap().state, TaskState::Paused);
    std::thread::spawn(move || drop(reservation))
        .join()
        .unwrap();
    assert_eq!(other.commit(prepared).unwrap().state, TaskState::Queued);
    other.resume(paused).unwrap();
}

#[test]
fn update_reservation_requires_no_queued_or_running_work() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    store.prepare(id, &new_task(dir.path())).unwrap();
    store.commit(id).unwrap();
    assert!(store.reserve_update_if_idle().unwrap().is_none());
    store.start(id).unwrap();
    assert!(store.reserve_update_if_idle().unwrap().is_none());
    store.pause(id).unwrap();
    assert!(store.reserve_update_if_idle().unwrap().is_some());
}

#[test]
fn concurrent_update_reservations_have_exactly_one_owner() {
    use std::sync::{Arc, Barrier};
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    Store::open(&database).unwrap();
    let start = Arc::new(Barrier::new(3));
    let acquired = Arc::new(Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let database = database.clone();
            let start = start.clone();
            let acquired = acquired.clone();
            std::thread::spawn(move || {
                let store = Store::open(&database).unwrap();
                start.wait();
                let guard = store.reserve_update_if_idle().unwrap();
                acquired.wait();
                guard.is_some()
            })
        })
        .collect();
    start.wait();
    acquired.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|owned| *owned)
            .count(),
        1
    );
}

#[cfg(windows)]
#[test]
fn installer_guard_also_blocks_store_work_admission() {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::CreateMutexW};
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let prepared = TaskId::new_v4();
    let paused = TaskId::new_v4();
    let queued = TaskId::new_v4();
    for id in [prepared, paused, queued] {
        store.prepare(id, &new_task(dir.path())).unwrap();
    }
    for id in [paused, queued] {
        store.commit(id).unwrap();
    }
    store.pause(paused).unwrap();
    let name: Vec<u16> = ndm_core::desktop_installation_namespace(dir.path())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let guard = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    assert!(!guard.is_null());
    let results = (
        store.commit(prepared),
        store.resume(paused),
        store.start(queued),
        store.recover_interrupted(),
    );
    unsafe {
        CloseHandle(guard);
    }
    assert_update_blocked(results.0);
    assert_update_blocked(results.1);
    assert_update_blocked(results.2);
    assert_update_blocked(results.3);
}

#[cfg(windows)]
#[test]
fn update_reservation_blocks_another_process_and_releases_after_drop() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let store = Store::open(&database).unwrap();
    let prepared = TaskId::new_v4();
    let paused = TaskId::new_v4();
    for id in [prepared, paused] {
        store.prepare(id, &new_task(dir.path())).unwrap();
    }
    store.commit(paused).unwrap();
    store.pause(paused).unwrap();
    let child = |blocked: bool| {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "update_admission_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("NDM_TEST_ADMISSION_DB", &database)
            .env("NDM_TEST_ADMISSION_PREPARED", prepared.to_string())
            .env("NDM_TEST_ADMISSION_PAUSED", paused.to_string())
            .env(
                "NDM_TEST_ADMISSION_BLOCKED",
                if blocked { "1" } else { "0" },
            )
            .output()
            .unwrap()
    };
    let reservation = store.reserve_update_if_idle().unwrap().unwrap();
    let blocked = child(true);
    assert!(blocked.status.success(), "{:?}", blocked);
    drop(reservation);
    let allowed = child(false);
    assert!(allowed.status.success(), "{:?}", allowed);
    assert_eq!(
        store.get(prepared).unwrap().unwrap().state,
        TaskState::Queued
    );
    assert_eq!(store.get(paused).unwrap().unwrap().state, TaskState::Queued);
}

#[cfg(windows)]
#[test]
#[ignore = "isolated subprocess fixture; harmless without scoped test environment"]
fn update_admission_fixture() {
    let Some(database) = std::env::var_os("NDM_TEST_ADMISSION_DB") else {
        return;
    };
    let database = std::path::PathBuf::from(database);
    let blocked = std::env::var("NDM_TEST_ADMISSION_BLOCKED").unwrap() == "1";
    let store = Store::open(&fs::canonicalize(&database).unwrap()).unwrap();
    let prepared =
        TaskId::parse_str(&std::env::var("NDM_TEST_ADMISSION_PREPARED").unwrap()).unwrap();
    let paused = TaskId::parse_str(&std::env::var("NDM_TEST_ADMISSION_PAUSED").unwrap()).unwrap();
    assert_eq!(
        ndm_core::desktop_installation_active(database.parent().unwrap()).unwrap(),
        blocked
    );
    if blocked {
        assert_update_blocked(store.commit(prepared));
        assert_update_blocked(store.resume(paused));
        assert!(store.reserve_update_if_idle().unwrap().is_none());
    } else {
        store.commit(prepared).unwrap();
        store.resume(paused).unwrap();
    }
}

#[test]
fn replacing_expired_link_is_limited_to_paused_web_tasks_and_is_secret_safe() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    let original = new_task(dir.path());
    store.prepare(id, &original).unwrap();
    assert!(
        store
            .update_web_url(id, "https://example.test/new")
            .is_err()
    );
    store.commit(id).unwrap();
    assert!(
        store
            .update_web_url(id, "https://example.test/new")
            .is_err()
    );
    store.pause(id).unwrap();
    for bad in [
        "ftp://example.test/file",
        "https://user:pass@example.test/file",
        "not a url",
    ] {
        assert!(store.update_web_url(id, bad).is_err());
    }
    store
        .update_web_url(id, "https://example.test/new?token=same-origin-secret")
        .unwrap();
    let same = store.get(id).unwrap().unwrap();
    assert_eq!(same.request.headers, original.headers);
    assert_eq!(same.request.source_url, original.source_url);
    assert_eq!(same.state, TaskState::Paused);
    store
        .update_web_url(
            id,
            "https://cdn.example.test/file?token=cross-origin-secret",
        )
        .unwrap();
    let cross = store.get(id).unwrap().unwrap();
    assert!(cross.request.headers.is_empty());
    assert!(cross.request.source_url.is_none());
    assert_eq!(cross.request.expected_size, original.expected_size);
    drop(store);
    for entry in fs::read_dir(dir.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = fs::read(path).unwrap();
            for secret in ["same-origin-secret", "cross-origin-secret"] {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes())
                );
            }
        }
    }
}

#[test]
fn replacing_auto_detected_links_preserves_download_kind() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    for (original, replacement, is_hls) in [
        (
            "https://example.test/live.m3u8",
            "https://cdn.test/renewed?id=1",
            true,
        ),
        (
            "https://example.test/file.zip",
            "https://cdn.test/file.m3u8",
            false,
        ),
    ] {
        let id = TaskId::new_v4();
        let mut request = new_task(dir.path());
        request.url = original.into();
        store.prepare(id, &request).unwrap();
        store.commit(id).unwrap();
        store.pause(id).unwrap();
        store.update_web_url(id, replacement).unwrap();
        let task = store.get(id).unwrap().unwrap();
        assert!(!matches!(task.options, TaskOptions::Auto));
        assert_eq!(matches!(task.options, TaskOptions::Hls(_)), is_hls);
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
fn replacing_dash_link_preserves_media_processing_and_selected_tracks() {
    let directory = TempDir::new().unwrap();
    let store = Store::open(&directory.path().join("tasks.db")).unwrap();
    for options in [
        TaskOptions::Auto,
        TaskOptions::Dash(ndm_core::DashOptions {
            representation_index: Some(1),
            audio_indices: vec![0, 1],
            ..Default::default()
        }),
    ] {
        let id = TaskId::new_v4();
        let mut request = new_task(directory.path());
        request.url = "https://example.test/VIDEO.MPD?token=old".into();
        store.prepare_with_options(id, &request, &options).unwrap();
        store.commit(id).unwrap();
        store.pause(id).unwrap();
        store
            .update_web_url(id, "https://cdn.test/renewed?id=1")
            .unwrap();
        let task = store.get(id).unwrap().unwrap();
        let TaskOptions::Dash(updated) = task.options else {
            panic!("DASH kind must survive a suffixless replacement URL")
        };
        if let TaskOptions::Dash(original) = options {
            assert_eq!(updated, original);
        }
        assert!(task.request.headers.is_empty());
        assert_eq!(task.request.expected_size, None);
    }
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
fn protocol_options_are_durable_encrypted_and_checked_for_idempotency() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("tasks.db");
    let id = TaskId::new_v4();
    let options = TaskOptions::Sftp(SftpOptions {
        trusted_fingerprint: Some("SHA256:example".into()),
        auth: Some(SftpAuth::Password("secret-sftp-password".into())),
        ..Default::default()
    });
    let store = Store::open(&database).unwrap();
    let request = new_task(dir.path());
    store.prepare_with_options(id, &request, &options).unwrap();
    store.prepare_with_options(id, &request, &options).unwrap();
    assert!(
        store
            .prepare_with_options(id, &request, &TaskOptions::Auto)
            .is_err()
    );
    drop(store);
    let restored = Store::open(&database).unwrap().get(id).unwrap().unwrap();
    assert_eq!(restored.options, options);
    for entry in fs::read_dir(dir.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            assert!(
                !fs::read(path)
                    .unwrap()
                    .windows(b"secret-sftp-password".len())
                    .any(|window| window == b"secret-sftp-password")
            );
        }
    }
}

#[test]
fn deleting_a_paused_record_preserves_unrelated_files() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("tasks.db")).unwrap();
    let id = TaskId::new_v4();
    store.prepare(id, &new_task(dir.path())).unwrap();
    store.commit(id).unwrap();
    store.pause(id).unwrap();
    let existing = dir.path().join("existing.bin");
    fs::write(&existing, b"preserve").unwrap();
    store.delete(id).unwrap();
    assert!(store.get(id).unwrap().is_none());
    assert_eq!(fs::read(existing).unwrap(), b"preserve");
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
