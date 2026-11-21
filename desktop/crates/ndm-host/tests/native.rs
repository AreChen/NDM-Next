use std::{
    fs::File,
    io::{Cursor, Write},
    sync::Arc,
};

use ndm_core::plugins::PluginRegistry;
use ndm_core::{Store, TaskId, TaskOptions, TaskState};
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
fn browser_context_exposes_versions_and_requires_an_eligible_page_engine() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path();
    let store = Arc::new(Store::open(&profile.join("tasks.db")).unwrap());
    let package = profile.join("page.zip");
    let mut archive = zip::ZipWriter::new(File::create(&package).unwrap());
    let file = zip::write::SimpleFileOptions::default();
    let engine = format!("page-engine{}", std::env::consts::EXE_SUFFIX);
    archive.start_file("plugin.json", file).unwrap();
    serde_json::to_writer(
        &mut archive,
        &json!({
            "schema": 1, "id": "page-fixture", "version": "0.2.0",
            "name": {"en":"Page fixture","zh":"网页插件"}, "description": {"en":"A fixture"},
            "engine": engine, "engine_protocol": 1, "limits": {"active_tasks": 1},
            "inputs": [{"schemes":["https"],"hosts":["video.test"]}],
            "manual_inputs": [{"schemes":["http"],"hosts":["media.test"]}],
            "capabilities": {"browser_page": true}
        }),
    )
    .unwrap();
    archive.start_file(&engine, file).unwrap();
    archive.write_all(b"fixture").unwrap();
    archive.finish().unwrap();
    PluginRegistry::new(profile).import(&package).unwrap();
    let host = Host::new(store.clone(), profile.join("downloads"));

    let page = "http://media.test/watch";
    let context = host.handle(json!({"type":"browser_context","page_url":page}));
    assert_eq!(context["desktop_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        context["plugins"],
        json!([{
            "id":"page-fixture", "name":{"en":"Page fixture","zh":"网页插件"}, "version":"0.2.0", "page":true
        }])
    );
    let id = TaskId::new_v4();
    let handoff = json!({"type":"prepare","id":id,"task":{"url":page,"plugin":"page-fixture"}});
    assert_eq!(host.handle(handoff.clone())["state"], "prepared");
    assert_eq!(host.handle(handoff)["state"], "prepared");
    assert!(matches!(
        store.get(id).unwrap().unwrap().options,
        TaskOptions::Plugin(options) if options.id == "page-fixture"
    ));
    let other = "http://example.test/watch";
    assert_eq!(
        host.handle(json!({"type":"browser_context","page_url":other}))["plugins"][0]["page"],
        false
    );
    assert_eq!(host.handle(json!({"type":"prepare","id":TaskId::new_v4(),"task":{"url":other,"plugin":"page-fixture"}}))["error"], "plugin_unavailable");
}

#[test]
fn changing_default_directory_keeps_existing_handoffs_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("tasks.db")).unwrap());
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    let id = TaskId::new_v4();
    assert_eq!(
        Host::new(store.clone(), first.clone()).handle(request(id))["ok"],
        true
    );
    let host = Host::new(store.clone(), second.clone());
    assert_eq!(host.handle(request(id))["ok"], true);
    assert_eq!(store.get(id).unwrap().unwrap().request.destination, first);
    let new_id = TaskId::new_v4();
    assert_eq!(host.handle(request(new_id))["ok"], true);
    assert_eq!(
        store.get(new_id).unwrap().unwrap().request.destination,
        second
    );
    let mut conflict = request(id);
    conflict["task"]["url"] = json!("https://example.test/other-file.zip");
    assert_eq!(host.handle(conflict)["ok"], false);
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
fn hls_mime_selects_media_processing_without_treating_playlist_size_as_video_size() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let host = Host::new(store.clone(), dir.path().join("downloads"));
    let id = TaskId::new_v4();
    let mut message = request(id);
    message["task"]["url"] = "https://example.test/playlist?id=4".into();
    message["task"]["media_type"] = "application/vnd.apple.mpegurl".into();
    assert_eq!(host.handle(message)["ok"], true);
    let task = store.get(id).unwrap().unwrap();
    assert!(matches!(task.options, TaskOptions::Hls(_)));
    assert_eq!(task.request.expected_size, None);
}

#[test]
fn dash_mime_selects_media_processing_without_treating_manifest_size_as_video_size() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
    let host = Host::new(store.clone(), dir.path().join("downloads"));
    let id = TaskId::new_v4();
    let mut message = request(id);
    message["task"]["url"] = "https://example.test/manifest?id=4".into();
    message["task"]["media_type"] = "application/dash+xml; charset=UTF-8".into();
    assert_eq!(host.handle(message)["ok"], true);
    let task = store.get(id).unwrap().unwrap();
    assert!(
        serde_json::to_value(task.options)
            .unwrap()
            .get("Dash")
            .is_some()
    );
    assert_eq!(task.request.expected_size, None);
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
#[test]
fn manifest_contains_only_validated_origins_and_preserves_unicode_paths() {
    let dir = tempfile::tempdir().unwrap();
    let host = dir.path().join("下载器").join("ndm-host.exe");
    let id = "caegebfdojcdlkkiaiokeoadefdooamd";
    let manifest = ndm_host::native_manifest("com.ndmnext.host", &host, &[id.into()]).unwrap();
    let text = serde_json::to_string(&manifest).unwrap();
    let decoded: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(decoded["path"], host.to_str().unwrap());
    assert_eq!(
        decoded["allowed_origins"][0],
        format!("chrome-extension://{id}/")
    );
    assert!(ndm_host::native_manifest("com.ndmnext.host", &host, &["*".into()]).is_err());
    assert!(ndm_host::native_manifest("invalid-name", &host, &[id.into()]).is_err());
    assert!(
        ndm_host::native_manifest(
            "com.ndmnext.host",
            std::path::Path::new("relative.exe"),
            &[id.into()]
        )
        .is_err()
    );
}
