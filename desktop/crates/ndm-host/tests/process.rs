use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

use ndm_host::{read_frame, write_frame};
use serde_json::{Value, json};

#[test]
fn host_process_exchanges_framed_messages_and_reopens_durable_storage() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let host_exe = bin.join("ndm-host.exe");
    fs::copy(env!("CARGO_BIN_EXE_ndm-host"), &host_exe).unwrap();
    // This test only prepares and aborts: a sentinel stands in for the app.
    fs::write(bin.join("ndm-desktop.exe"), b"sentinel").unwrap();
    let mut child = Command::new(host_exe)
        .env("LOCALAPPDATA", dir.path().join("profile"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let id = ndm_core::TaskId::new_v4().to_string();
    let commands = [
        (json!({"type":"status"}), "ok", json!(true)),
        (
            json!({"type":"prepare","id":id,"task":{"url":"https://example.test/file","source_url":null,"suggested_filename":null,"expected_size":null,"headers":{}}}),
            "state",
            json!("prepared"),
        ),
        (
            json!({"type":"inspect","id":id}),
            "state",
            json!("prepared"),
        ),
        (json!({"type":"abort","id":id}), "ok", json!(true)),
        (json!({"type":"inspect","id":id}), "state", json!("missing")),
    ];
    for (command, key, expected) in commands {
        write_frame(&mut input, &serde_json::to_vec(&command).unwrap()).unwrap();
        input.flush().unwrap();
        let reply: Value =
            serde_json::from_slice(&read_frame(&mut output).unwrap().unwrap()).unwrap();
        assert_eq!(reply[key], expected);
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[cfg(windows)]
#[test]
fn process_status_detects_a_desktop_instance_for_the_same_profile() {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::CreateMutexW};

    let dir = tempfile::tempdir().unwrap();
    let status = || {
        Command::new(env!("CARGO_BIN_EXE_ndm-host"))
            .arg("--desktop-running")
            .env("LOCALAPPDATA", dir.path())
            .status()
            .unwrap()
            .code()
            .unwrap()
    };
    assert_eq!(status(), 0);
    let name = format!(
        "{}-mutex",
        ndm_core::desktop_instance_namespace(&dir.path().join("NDM Next"))
    );
    let wide_name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: The name is NUL-terminated, and the created handle is closed below.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, wide_name.as_ptr()) };
    assert!(!mutex.is_null());
    assert_eq!(status(), 1);
    // SAFETY: The handle was just created and is still open.
    unsafe { CloseHandle(mutex) };
    assert_eq!(status(), 0);
}
