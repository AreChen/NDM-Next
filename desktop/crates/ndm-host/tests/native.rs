use std::{io::Cursor, sync::Arc};

use ndm_core::{Store, TaskId, TaskState};
use ndm_host::{Host, MAX_MESSAGE_BYTES, read_frame, write_frame};
use serde_json::{Value, json};

fn request(id: TaskId) -> Value {
    json!({"type":"prepare", "id":id.to_string(), "task":{
        "url":"https://example.test/file.zip", "source_url":"https://page.test/",
        "suggested_filename":"file.zip", "expected_size":123,
        "headers":{"cookie":"session=private", "authorization":"Bearer private"}
    }})
}

#[test]
fn native_frames_are_little_endian_and_bounded() {
    let payload = br#"{"type":"status"}"#;
    let mut framed = Vec::new();
    write_frame(&mut framed, payload).unwrap();
    assert_eq!(&framed[..4], &(payload.len() as u32).to_le_bytes());
    assert_eq!(
        read_frame(&mut Cursor::new(framed)).unwrap().unwrap(),
        payload
    );
    assert!(
        read_frame(&mut Cursor::new(
            ((MAX_MESSAGE_BYTES + 1) as u32).to_le_bytes()
        ))
        .is_err()
    );
    assert!(read_frame(&mut Cursor::new([3, 0, 0, 0, b'{'])).is_err());
    assert!(
        read_frame(&mut Cursor::new(Vec::<u8>::new()))
            .unwrap()
            .is_none()
    );
}

#[test]
fn prepare_is_durable_idempotent_and_does_not_start_until_commit() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tasks.db");
    let id = TaskId::new_v4();
    let host = Host::new(
        Arc::new(Store::open(&db).unwrap()),
        dir.path().join("downloads"),
    );
    assert_eq!(host.handle(request(id))["state"], "prepared");
    assert_eq!(host.handle(request(id))["state"], "prepared");
    drop(host);
    let reopened = Arc::new(Store::open(&db).unwrap());
    assert_eq!(
        reopened.get(id).unwrap().unwrap().state,
        TaskState::Prepared
    );
    let host = Host::new(reopened.clone(), dir.path().join("downloads"));
    assert_eq!(
        host.handle(json!({"type":"commit","id":id.to_string()}))["state"],
        "queued"
    );
    assert_eq!(
        host.handle(json!({"type":"commit","id":id.to_string()}))["state"],
        "queued"
    );
    assert_eq!(reopened.get(id).unwrap().unwrap().state, TaskState::Queued);
}

#[test]
fn conflict_abort_and_invalid_messages_do_not_start_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let host = Host::new(store.clone(), dir.path().join("downloads"));
    let id = TaskId::new_v4();
    assert_eq!(host.handle(request(id))["ok"], true);
    let mut conflict = request(id);
    conflict["task"]["url"] = "https://different.test/file".into();
    assert_eq!(host.handle(conflict)["ok"], false);
    assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Prepared);
    assert_eq!(
        host.handle(json!({"type":"abort","id":id.to_string()}))["ok"],
        true
    );
    assert!(store.get(id).unwrap().is_none());
    assert_eq!(
        host.handle(json!({"type":"commit","id":id.to_string()}))["ok"],
        false
    );
    for invalid in [
        json!({"type":"prepare","id":"not-a-uuid","task":{}}),
        json!({"type":"commit","id":id.to_string(),"extra":true}),
    ] {
        assert_eq!(host.handle(invalid)["ok"], false);
    }
}

#[test]
fn invalid_urls_and_headers_are_rejected_without_echoing_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let host = Host::new(store.clone(), dir.path().join("downloads"));
    let id = TaskId::new_v4();
    for (key, value) in [
        ("url", "file:///C:/private"),
        ("url", "https://user:password@example.test/file"),
        ("url", "javascript:alert(1)"),
    ] {
        let mut invalid = request(id);
        invalid["task"][key] = value.into();
        assert_eq!(host.handle(invalid)["ok"], false);
    }
    let mut invalid = request(id);
    invalid["task"]["headers"]["x-evil"] = "secret".into();
    let reply = host.handle(invalid);
    assert_eq!(reply["ok"], false);
    assert!(!reply.to_string().contains("secret"));
    assert!(store.get(id).unwrap().is_none());
}
