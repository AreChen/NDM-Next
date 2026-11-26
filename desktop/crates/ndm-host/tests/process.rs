use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

use ndm_host::{read_frame, write_frame};
use serde_json::{Value, json};

const HOST_BINARY: &str = env!("CARGO_BIN_EXE_ndm-host");

fn host_fixture_directory() -> tempfile::TempDir {
    #[cfg(target_os = "linux")]
    {
        // Linux can reject a just-copied executable while a parallel spawn
        // still has its writable file descriptor. Stage on the build volume
        // so fixtures can link the existing, already closed binary instead.
        let parent = std::path::Path::new(HOST_BINARY).parent().unwrap();
        tempfile::tempdir_in(parent).unwrap()
    }
    #[cfg(not(target_os = "linux"))]
    tempfile::tempdir().unwrap()
}

fn stage_host(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    fs::hard_link(HOST_BINARY, path).unwrap();
    #[cfg(not(target_os = "linux"))]
    fs::copy(HOST_BINARY, path).unwrap();
}

#[test]
fn rewriting_manifest_preserves_existing_valid_origins_for_the_same_host() {
    let dir = host_fixture_directory();
    let host = dir.path().join("ndm-host.exe");
    stage_host(&host);
    let first = "caegebfdojcdlkkiaiokeoadefdooamd";
    let added = "pdkhpmalkocjfkfopjhmopikegpmcbcf";
    let edge = "hiblommiheggidolemmnnlocmocebckp";
    let write = |preserve: bool, ids: &[&str]| {
        Command::new(&host)
            .arg("--write-manifest")
            .arg("com.ndmnext.host")
            .args(if preserve {
                vec!["--preserve-origins"]
            } else {
                vec![]
            })
            .args(ids)
            .status()
            .unwrap()
    };
    assert!(write(false, &[first, added]).success());
    let path = dir.path().join("native-messaging-host.json");
    let mut existing: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    existing["allowed_origins"]
        .as_array_mut()
        .unwrap()
        .push(json!("*"));
    fs::write(&path, serde_json::to_vec(&existing).unwrap()).unwrap();
    assert!(write(true, &[first, edge]).success());
    let updated: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        updated["allowed_origins"],
        json!([
            format!("chrome-extension://{first}/"),
            format!("chrome-extension://{edge}/"),
            format!("chrome-extension://{added}/")
        ])
    );
    let mut other_host = updated;
    other_host["path"] = json!(dir.path().join("another-host.exe"));
    fs::write(&path, serde_json::to_vec(&other_host).unwrap()).unwrap();
    assert!(write(true, &[first, edge]).success());
    let replaced: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        replaced["allowed_origins"],
        json!([
            format!("chrome-extension://{first}/"),
            format!("chrome-extension://{edge}/")
        ])
    );
    assert!(write(false, &[first]).success());
    let reset: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        reset["allowed_origins"],
        json!([format!("chrome-extension://{first}/")])
    );
}

#[test]
fn relocating_integration_preserves_only_the_identified_installations_valid_origins() {
    let dir = host_fixture_directory();
    let old = dir.path().join("previous-install");
    let new = dir.path().join("new-install");
    fs::create_dir_all(&old).unwrap();
    fs::create_dir_all(&new).unwrap();
    let old_host = old.join("ndm-host.exe");
    let host = new.join("ndm-host.exe");
    stage_host(&old_host);
    stage_host(&host);
    let old_desktop = old.join("ndm-desktop.exe");
    fs::write(&old_desktop, b"identified desktop installation").unwrap();
    let name = ndm_host::LEGACY_BROWSER_HOST_NAME;
    let primary = ndm_host::BROWSER_HOST_NAME;
    let default = "caegebfdojcdlkkiaiokeoadefdooamd";
    let prior = "abcdefghijklmnopabcdefghijklmnop";
    let existing = "pdkhpmalkocjfkfopjhmopikegpmcbcf";
    let prior_manifest = old.join("native-messaging-host.json");
    let requested = || {
        Command::new(&host)
            .args([
                "--write-manifest",
                name,
                "--primary-host",
                primary,
                "--preserve-origins",
                "--migrate-origins-from",
            ])
            .arg(&old_desktop)
            .arg(default)
            .status()
            .unwrap()
    };
    let initial = ndm_host::native_manifest(name, &old_host, &[prior.into()]).unwrap();
    let mut imported = initial.clone();
    imported["allowed_origins"]
        .as_array_mut()
        .unwrap()
        .push(json!("*"));
    fs::write(&prior_manifest, serde_json::to_vec(&imported).unwrap()).unwrap();
    let own = ndm_host::native_manifest(primary, &host, &[existing.into()]).unwrap();
    fs::write(
        new.join("onesdm-native-messaging-host.json"),
        serde_json::to_vec(&own).unwrap(),
    )
    .unwrap();
    let manifest_paths = [
        (name, new.join("native-messaging-host.json")),
        (primary, new.join("onesdm-native-messaging-host.json")),
    ];
    let check_manifests = |origins: Value| {
        for (name, path) in &manifest_paths {
            let manifest: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            assert_eq!(manifest["name"], *name);
            assert_eq!(
                fs::canonicalize(manifest["path"].as_str().unwrap()).unwrap(),
                fs::canonicalize(&host).unwrap()
            );
            assert_eq!(manifest["allowed_origins"], origins);
        }
    };
    assert!(requested().success());
    check_manifests(json!([
        format!("chrome-extension://{default}/"),
        format!("chrome-extension://{existing}/"),
        format!("chrome-extension://{prior}/")
    ]));
    for (field, value) in [
        ("name", json!("com.unrelated.host")),
        ("path", json!(host)),
        ("type", json!("unrelated")),
    ] {
        let mut unrelated = initial.clone();
        unrelated[field] = value;
        fs::write(&prior_manifest, serde_json::to_vec(&unrelated).unwrap()).unwrap();
        for (_, path) in &manifest_paths {
            fs::remove_file(path).unwrap();
        }
        assert!(requested().success());
        check_manifests(json!([format!("chrome-extension://{default}/")]));
    }
    assert!(
        !Command::new(&host)
            .args([
                "--write-manifest",
                name,
                "--preserve-origins",
                "--migrate-origins-from"
            ])
            .arg(&prior_manifest)
            .arg(default)
            .status()
            .unwrap()
            .success(),
        "arbitrary manifest paths must not identify installations"
    );
}

#[test]
fn host_process_exchanges_framed_messages_and_reopens_durable_storage() {
    let dir = host_fixture_directory();
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let host_exe = bin.join("ndm-host.exe");
    stage_host(&host_exe);
    // This test only prepares and aborts: a sentinel stands in for the app.
    fs::write(
        bin.join(if cfg!(windows) {
            "ndm-desktop.exe"
        } else {
            "ndm-desktop"
        }),
        b"sentinel",
    )
    .unwrap();
    let mut command = Command::new(host_exe);
    command.env("LOCALAPPDATA", dir.path().join("profile"));
    #[cfg(target_os = "macos")]
    command.env("HOME", dir.path().join("home"));
    #[cfg(target_os = "linux")]
    command
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_RUNTIME_DIR", dir.path());
    let mut child = command
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

#[cfg(windows)]
#[test]
fn installation_namespace_command_returns_profile_guard_without_newline() {
    let dir = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_ndm-host"))
        .arg("--installation-namespace")
        .env("LOCALAPPDATA", dir.path())
        .output()
        .unwrap();
    assert!(result.status.success());
    let expected = format!(
        "{}-installing",
        ndm_core::desktop_instance_namespace(&dir.path().join("NDM Next"))
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), expected);
    assert!(!dir.path().join("NDM Next/tasks.db").exists());
}

#[cfg(windows)]
#[test]
fn native_host_does_not_open_storage_during_installation() {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::CreateMutexW};
    let dir = tempfile::tempdir().unwrap();
    let name: Vec<u16> = format!(
        "{}-installing",
        ndm_core::desktop_instance_namespace(&dir.path().join("NDM Next"))
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    // SAFETY: This test creates and closes the guard for an isolated profile.
    let guard = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    assert!(!guard.is_null());
    let result = Command::new(env!("CARGO_BIN_EXE_ndm-host"))
        .env("LOCALAPPDATA", dir.path())
        .output()
        .unwrap();
    unsafe {
        CloseHandle(guard);
    }
    assert!(result.stdout.is_empty());
    assert!(
        !dir.path().join("NDM Next/tasks.db").exists(),
        "native host must not open task storage during replacement"
    );
}

#[cfg(windows)]
mod shutdown {
    use super::*;
    use std::{
        path::{Path, PathBuf},
        process::Child,
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, CreateMutexW, ResetEvent, WaitForSingleObject},
    };

    struct Fixture {
        child: Child,
        marker: PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    fn fixture(image: &Path, profile: &Path, marker: &Path, mode: &str) -> Fixture {
        fs::create_dir_all(image.parent().unwrap()).unwrap();
        if !image.exists() {
            fs::copy(std::env::current_exe().unwrap(), image).unwrap();
        }
        let child = Command::new(image)
            .args([
                "--exact",
                "shutdown::fixture_process",
                "--ignored",
                "--nocapture",
            ])
            .env("NDM_TEST_SHUTDOWN_PROFILE", profile)
            .env("NDM_TEST_SHUTDOWN_MARKER", marker)
            .env("NDM_TEST_SHUTDOWN_MODE", mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let result = Fixture {
            child,
            marker: marker.to_owned(),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "isolated fixture did not initialize");
        result
    }
    fn helper(image: &Path, profile: &Path) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_ndm-host"))
            .arg("--shutdown-desktop")
            .arg(image)
            .arg("--data-dir")
            .arg(profile)
            .output()
            .unwrap()
    }
    #[test]
    #[ignore = "only launched as an isolated child by shutdown tests"]
    fn fixture_process() {
        let Some(profile) = std::env::var_os("NDM_TEST_SHUTDOWN_PROFILE") else {
            return;
        };
        let marker = PathBuf::from(std::env::var_os("NDM_TEST_SHUTDOWN_MARKER").unwrap());
        let mode = std::env::var("NDM_TEST_SHUTDOWN_MODE").unwrap();
        let name = ndm_core::desktop_instance_namespace(Path::new(&profile));
        let wide = |suffix: &str| {
            format!("{name}-{suffix}")
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, wide("mutex").as_ptr());
            let ready = CreateEventW(std::ptr::null(), 1, 1, wide("ready").as_ptr());
            let event = if mode == "legacy" {
                std::ptr::null_mut()
            } else {
                CreateEventW(std::ptr::null(), 0, 0, wide("shutdown").as_ptr())
            };
            assert!(!mutex.is_null() && !ready.is_null());
            let _media_child = std::env::var_os("NDM_TEST_MEDIA_IMAGE").map(|image| {
                let media_marker = marker.with_extension("media");
                let child = Command::new(image)
                    .args([
                        "--exact",
                        "shutdown::fixture_process",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env_remove("NDM_TEST_MEDIA_IMAGE")
                    .env("NDM_TEST_SHUTDOWN_MODE", "legacy")
                    .env(
                        "NDM_TEST_SHUTDOWN_PROFILE",
                        Path::new(&profile).join("media"),
                    )
                    .env("NDM_TEST_SHUTDOWN_MARKER", &media_marker)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !media_marker.exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(media_marker.exists());
                child
            });
            fs::write(&marker, std::process::id().to_string()).unwrap();
            if event.is_null() {
                thread::sleep(Duration::from_secs(30));
            } else if WaitForSingleObject(event, 30_000) == WAIT_OBJECT_0 {
                ResetEvent(ready);
                fs::write(marker.with_extension("requested"), b"shutdown").unwrap();
                if mode == "hang" {
                    thread::sleep(Duration::from_secs(30));
                }
            }
            if !event.is_null() {
                CloseHandle(event);
            }
            CloseHandle(ready);
            CloseHandle(mutex);
        }
    }
    #[test]
    fn shutdown_command_signals_and_waits_for_matching_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("install/ndm-desktop.exe");
        let profile = dir.path().join("profile/NDM Next");
        let mut child = fixture(
            &image,
            &profile,
            &dir.path().join("desktop.ready"),
            "graceful",
        );
        let result = helper(&image, &profile);
        assert!(
            child.marker.with_extension("requested").exists(),
            "helper did not request graceful shutdown"
        );
        assert!(result.status.success(), "{:?}", result);
        assert!(
            child.child.try_wait().unwrap().is_some(),
            "helper must wait for the desktop to exit"
        );
    }

    #[test]
    fn legacy_shutdown_only_terminates_the_canonical_installation_image() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile/NDM Next");
        let image = dir.path().join("install/ndm-desktop.exe");
        let mut matching = fixture(
            &image,
            &profile,
            &dir.path().join("matching.ready"),
            "legacy",
        );
        let mut unrelated = fixture(
            &dir.path().join("other/ndm-desktop.exe"),
            &dir.path().join("other-profile"),
            &dir.path().join("unrelated.ready"),
            "legacy",
        );
        let result = helper(&image, &profile);
        assert!(result.status.success(), "{:?}", result);
        assert!(matching.child.try_wait().unwrap().is_some());
        assert!(
            unrelated.child.try_wait().unwrap().is_none(),
            "same basename in another directory must survive"
        );
    }

    #[test]
    fn graceful_timeout_preserves_the_active_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("install/ndm-desktop.exe");
        let profile = dir.path().join("profile/NDM Next");
        let mut matching = fixture(&image, &profile, &dir.path().join("matching.ready"), "hang");
        let error =
            ndm_core::shutdown_desktop_installation(&profile, &image, Duration::from_millis(150))
                .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(matching.marker.with_extension("requested").exists());
        assert!(
            matching.child.try_wait().unwrap().is_none(),
            "graceful timeout must not terminate active jobs"
        );
    }

    #[test]
    fn a_different_installation_owning_the_profile_is_not_signaled() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("install/ndm-desktop.exe");
        fs::create_dir_all(image.parent().unwrap()).unwrap();
        fs::write(&image, b"not running").unwrap();
        let profile = dir.path().join("profile/NDM Next");
        let mut unrelated = fixture(
            &dir.path().join("other/ndm-desktop.exe"),
            &profile,
            &dir.path().join("other.ready"),
            "graceful",
        );
        assert_eq!(helper(&image, &profile).status.code(), Some(3));
        assert!(!unrelated.marker.with_extension("requested").exists());
        assert!(unrelated.child.try_wait().unwrap().is_none());
    }

    #[test]
    fn relative_or_unexpected_desktop_image_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile/NDM Next");
        assert!(
            !helper(Path::new("ndm-desktop.exe"), &profile)
                .status
                .success()
        );
        assert!(
            !helper(&dir.path().join("other.exe"), &profile)
                .status
                .success()
        );
    }

    #[test]
    fn shutdown_releases_only_installed_hosts_and_descendant_media() {
        // Windows termination is asynchronous; exercise the retained host in
        // consecutive collection passes while its image may be tearing down.
        for _ in 0..3 {
            assert_shutdown_releases_only_installed_hosts_and_descendant_media();
        }
    }

    fn assert_shutdown_releases_only_installed_hosts_and_descendant_media() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("install/ndm-desktop.exe");
        let media = dir.path().join("install/ffmpeg/ffmpeg.exe");
        fs::create_dir_all(media.parent().unwrap()).unwrap();
        fs::copy(std::env::current_exe().unwrap(), &media).unwrap();
        // Independent tools at the same exact image are not owned by the desktop.
        let mut independent = fixture(
            &media,
            &dir.path().join("tool-profile"),
            &dir.path().join("tool.ready"),
            "legacy",
        );
        fs::copy(std::env::current_exe().unwrap(), &image).unwrap();
        let profile = dir.path().join("profile/NDM Next");
        let marker = dir.path().join("desktop.ready");
        let child = Command::new(&image)
            .args([
                "--exact",
                "shutdown::fixture_process",
                "--ignored",
                "--nocapture",
            ])
            .env("NDM_TEST_SHUTDOWN_PROFILE", &profile)
            .env("NDM_TEST_SHUTDOWN_MARKER", &marker)
            .env("NDM_TEST_SHUTDOWN_MODE", "graceful")
            .env("NDM_TEST_MEDIA_IMAGE", &media)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut desktop = Fixture {
            child,
            marker: marker.clone(),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists());
        let media_pid: u32 = fs::read_to_string(marker.with_extension("media"))
            .unwrap()
            .parse()
            .unwrap();
        let media_handle = unsafe {
            windows_sys::Win32::System::Threading::OpenProcess(
                windows_sys::Win32::System::Threading::SYNCHRONIZATION_SYNCHRONIZE,
                0,
                media_pid,
            )
        };
        assert!(!media_handle.is_null());
        let host_image = dir.path().join("install/ndm-host.exe");
        fs::copy(env!("CARGO_BIN_EXE_ndm-host"), &host_image).unwrap();
        let host_child = Command::new(&host_image)
            .env("LOCALAPPDATA", dir.path().join("host-profile"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut host = Fixture {
            child: host_child,
            marker: dir.path().join("unused"),
        };
        let unrelated_host_image = dir.path().join("other/ndm-host.exe");
        fs::create_dir_all(unrelated_host_image.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_ndm-host"), &unrelated_host_image).unwrap();
        let unrelated_host_child = Command::new(unrelated_host_image)
            .env("LOCALAPPDATA", dir.path().join("other-host-profile"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut unrelated_host = Fixture {
            child: unrelated_host_child,
            marker: dir.path().join("unused"),
        };
        let result = helper(&image, &profile);
        let media_exited = unsafe { WaitForSingleObject(media_handle, 1_000) } == WAIT_OBJECT_0;
        unsafe {
            CloseHandle(media_handle);
        }
        assert!(result.status.success(), "{:?}", result);
        assert!(desktop.child.try_wait().unwrap().is_some());
        assert!(
            host.child.try_wait().unwrap().is_some(),
            "installation native host must release its executable"
        );
        assert!(
            media_exited,
            "desktop's media child must release its executable"
        );
        assert!(
            independent.child.try_wait().unwrap().is_none(),
            "independent media at the same path must survive"
        );
        assert!(
            unrelated_host.child.try_wait().unwrap().is_none(),
            "other installation's native host must survive"
        );
    }
}
