use super::*;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

fn release(version: &str) -> Value {
    let name = asset_name(&Version::parse(version).unwrap());
    json!({"tag_name": format!("desktop-v{version}"), "draft": false, "prerelease": false,
        "assets": [{"name": name, "size": 9, "digest": format!("sha256:{}", "ab".repeat(32)),
        "browser_download_url": format!("https://github.com/AreChen/NDM-Next/releases/download/desktop-v{version}/{name}")}]})
}
fn select(releases: Value) -> Option<UpdateRelease> {
    find_update(
        &serde_json::to_vec(&releases).unwrap(),
        &Version::parse("0.1.0").unwrap(),
    )
    .unwrap()
}

#[test]
fn bridge_accepts_branded_packages_before_and_after_the_repository_rename() {
    for repo in ["NDM-Next", "onesdm", "other"] {
        let mut data = release("0.7.0");
        let asset = &mut data["assets"][0];
        let name = asset["name"].as_str().unwrap();
        assert!(name.starts_with("OnesDM"));
        asset["browser_download_url"] = json!(format!(
            "https://github.com/AreChen/{repo}/releases/download/desktop-v0.7.0/{name}"
        ));
        assert_eq!(select(json!([data])).is_some(), repo != "other");
    }
}

#[test]
fn automatic_checks_remain_infrequent_across_restarts_and_failures() {
    assert!(automatic_check_due(None, 1_000));
    assert!(!automatic_check_due(Some(1_000), 1_001));
    assert!(!automatic_check_due(Some(1_000), 44_199));
    assert!(automatic_check_due(Some(1_000), 44_200));
    // A clock correction repairs the persisted timestamp rather than disabling checks forever.
    assert!(automatic_check_due(Some(100_000), 1_000));
}

#[test]
fn cached_hints_ignore_current_older_prerelease_and_invalid_versions() {
    for hint in [
        None,
        Some(env!("CARGO_PKG_VERSION")),
        Some("0.0.1"),
        Some("99.0.0-beta.1"),
        Some("99.0.0+build"),
        Some("not a version"),
    ] {
        assert!(remembered_version(hint).is_none());
    }
    assert_eq!(
        remembered_version(Some("99.0.0")).unwrap().to_string(),
        "99.0.0"
    );
}

#[test]
fn discovering_or_caching_an_update_never_authorizes_installation() {
    let release = select(json!([release("0.2.0")])).unwrap();
    let ready = UpdateState::Ready(ReadyUpdate {
        release: release.clone(),
        path: PathBuf::from("cached.exe"),
    });
    assert!(!ready.installation_requested(false));
    assert!(ready.installation_requested(true));
    assert!(!UpdateState::Available(release).installation_requested(true));
    assert!(!UpdateState::Downloading.installation_requested(true));
}
#[test]
fn does_not_offer_a_release_without_an_installable_verified_asset() {
    assert!(
        select(json!([{"tag_name":"desktop-v0.2.0","draft":false,"prerelease":false,"assets":[]}]))
            .is_none()
    );
}
#[test]
fn selects_highest_stable_desktop_version_and_skips_extension_versions() {
    let mut extension = release("99.0.0");
    extension["tag_name"] = json!("v99.0.0");
    let mut draft = release("9.0.0");
    draft["draft"] = json!(true);
    let mut pre = release("8.0.0");
    pre["prerelease"] = json!(true);
    let selected = select(json!([
        release("0.1.0"),
        release("0.2.0"),
        extension,
        draft,
        pre,
        release("0.3.0")
    ]))
    .unwrap();
    assert_eq!(selected.version.to_string(), "0.3.0");
    assert!(
        select(json!([
            release("0.1.0"),
            release("0.0.9"),
            release("2.0.0-beta.1"),
            release("2.0.0+build")
        ]))
        .is_none()
    );
}
#[test]
fn rejects_wrong_assets_origins_digests_and_sizes() {
    for (field, value) in [
        ("name", json!("other.exe")),
        (
            "browser_download_url",
            json!("https://example.test/NDM-Next-Setup-0.2.0-x64.exe"),
        ),
        (
            "browser_download_url",
            json!(
                "https://github.com/another/repository/releases/download/desktop-v0.2.0/NDM-Next-Setup-0.2.0-x64.exe"
            ),
        ),
        ("digest", json!(null)),
        ("digest", json!("sha256:12")),
        ("digest", json!(format!("sha256:{}", "gg".repeat(32)))),
        ("size", json!(0)),
        ("size", json!(MAX_INSTALLER + 1)),
    ] {
        let mut invalid = release("0.2.0");
        invalid["assets"][0][field] = value;
        assert!(
            select(json!([invalid])).is_none(),
            "accepted invalid {field}"
        );
    }
    let mut duplicate = release("0.2.0");
    let copy = duplicate["assets"][0].clone();
    duplicate["assets"].as_array_mut().unwrap().push(copy);
    assert!(select(json!([duplicate])).is_none());
}
#[test]
fn redirects_require_exact_trusted_https_hosts_without_embedded_credentials() {
    for url in [
        "https://github.com/a",
        "https://release-assets.githubusercontent.com/a?signature=opaque",
        "https://objects.githubusercontent.com/a",
    ] {
        assert!(trusted_download_url(&url.parse().unwrap()));
    }
    for url in [
        "http://github.com/a",
        "https://github.com.evil.test/a",
        "https://user:secret@github.com/a",
        "https://github.com:8443/a",
        "file:///tmp/a.exe",
    ] {
        assert!(
            !trusted_download_url(&url.parse().unwrap()),
            "accepted {url}"
        );
    }
}
async fn response(body: &'static [u8], advertised: usize) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 2048];
        let _ = socket.read(&mut request).await;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {advertised}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        socket.write_all(body).await.unwrap();
    });
    format!("http://{address}/installer")
}
async fn receive_installer(
    url: String,
    mut release: UpdateRelease,
    directory: &std::path::Path,
) -> Result<ReadyUpdate, UpdateFailure> {
    release.url = url;
    let transfer = UpdateTransfer::new(release.bytes);
    download_with_client(
        release,
        directory.to_owned(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        4,
        transfer,
    )
    .await
}

fn fixture(bytes: &[u8]) -> UpdateRelease {
    let mut release = select(json!([release("0.2.0")])).unwrap();
    release.bytes = bytes.len() as u64;
    release.sha256 = Sha256::digest(bytes).into();
    release
}
#[tokio::test]
async fn publishes_only_verified_installer_and_rechecks_before_launch() {
    let directory = tempfile::tempdir().unwrap();
    let body = b"fake installer bytes";
    let ready = receive_installer(
        response(body, body.len()).await,
        fixture(body),
        directory.path(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&ready.path).unwrap(), body);
    verify_cached(&ready).await.unwrap();
    std::fs::write(&ready.path, b"changed same length!").unwrap();
    assert!(verify_cached(&ready).await.is_err());
}
#[tokio::test]
async fn bad_digest_truncation_and_wrong_lengths_never_publish_an_executable() {
    for (body, advertised, expected) in [
        (&b"wrong digest"[..], 12, &b"right digest"[..]),
        (&b"short"[..], 12, &b"right digest"[..]),
        (&b"other length"[..], 12, &b"longer expected body"[..]),
    ] {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            receive_installer(
                response(body, advertised).await,
                fixture(expected),
                directory.path()
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
#[ignore = "requires a published GitHub desktop release; downloads only, never installs"]
async fn published_github_installer_downloads_and_verifies_without_execution() {
    let proxy = std::env::var("NDM_UPDATE_TEST_PROXY")
        .ok()
        .map(|url| ndm_core::ProxyOptions {
            url,
            username: None,
            password: None,
        });
    let release = check_version(proxy.clone(), &Version::parse("0.0.0").unwrap())
        .await
        .unwrap()
        .expect("published desktop installer");
    let directory = tempfile::tempdir().unwrap();
    let transfer = UpdateTransfer::new(release.bytes);
    let ready = download(
        release.clone(),
        directory.path().to_owned(),
        proxy,
        16,
        transfer.clone(),
    )
    .await
    .unwrap();
    verify_cached(&ready).await.unwrap();
    println!(
        "Verified desktop {}: {} bytes, {:x}; {} download segments",
        ready.release.version,
        ready.release.bytes,
        Sha256::digest(std::fs::read(&ready.path).unwrap()),
        transfer.progress.snapshot().segments.len()
    );
}

#[tokio::test]
async fn updater_uses_parallel_ranges_and_live_progress_then_reuses_verified_cache() {
    use std::sync::atomic::AtomicUsize;
    let body = Arc::new(vec![73u8; 128 * 1024]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let server = {
        let (body, requests, active, peak) =
            (body.clone(), requests.clone(), active.clone(), peak.clone());
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (body, requests, active, peak) =
                    (body.clone(), requests.clone(), active.clone(), peak.clone());
                tokio::spawn(async move {
                    let mut raw = Vec::new();
                    let mut buffer = [0; 2048];
                    while !raw.windows(4).any(|part| part == b"\r\n\r\n") {
                        let n = socket.read(&mut buffer).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        raw.extend_from_slice(&buffer[..n]);
                    }
                    let request = String::from_utf8(raw).unwrap().to_ascii_lowercase();
                    let range = request
                        .lines()
                        .find_map(|line| line.strip_prefix("range: bytes="))
                        .unwrap();
                    let (start, end) = range.trim().split_once('-').unwrap();
                    let (start, end): (usize, usize) =
                        (start.parse().unwrap(), end.parse().unwrap());
                    requests.fetch_add(1, Ordering::SeqCst);
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    socket.write_all(format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nETag: \"stable\"\r\nConnection: close\r\n\r\n",end-start+1,body.len()).as_bytes()).await.unwrap();
                    for chunk in body[start..=end].chunks(1024) {
                        socket.write_all(chunk).await.unwrap();
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        })
    };
    let directory = tempfile::tempdir().unwrap();
    let mut release = fixture(&body);
    release.url = format!("http://{address}/installer");
    let transfer = UpdateTransfer::new(release.bytes);
    let task = tokio::spawn(download_with_client(
        release.clone(),
        directory.path().to_owned(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        4,
        transfer.clone(),
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = transfer.progress.snapshot();
            if state.downloaded > 0 && state.downloaded < release.bytes {
                break;
            }
            assert!(
                !task.is_finished(),
                "transfer finished without publishing intermediate progress"
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    let ready = task.await.unwrap().unwrap();
    assert_eq!(std::fs::read(&ready.path).unwrap(), *body);
    assert!(peak.load(Ordering::SeqCst) > 1);
    assert_eq!(requests.load(Ordering::SeqCst), 5);
    assert_eq!(transfer.progress.snapshot().downloaded, release.bytes);
    assert!(transfer.is_verifying());
    let cached = download_with_client(
        release.clone(),
        directory.path().to_owned(),
        reqwest::Client::new(),
        4,
        UpdateTransfer::new(release.bytes),
    )
    .await
    .unwrap();
    assert_eq!(cached.path, ready.path);
    assert_eq!(requests.load(Ordering::SeqCst), 5);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    let wrong_directory = tempfile::tempdir().unwrap();
    let mut wrong = release.clone();
    wrong.bytes += 1;
    let result = download_with_client(
        wrong.clone(),
        wrong_directory.path().to_owned(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        4,
        UpdateTransfer::new(wrong.bytes),
    )
    .await;
    assert_eq!(result, Err(UpdateFailure::Integrity));
    assert_eq!(
        requests.load(Ordering::SeqCst),
        6,
        "a mismatched range total must fail before segment requests"
    );
    assert_eq!(
        std::fs::read_dir(wrong_directory.path()).unwrap().count(),
        0
    );
    server.abort();
}

#[cfg(windows)]
#[test]
#[ignore = "subprocess fixture only"]
fn installer_handoff_fixture() {
    if let Ok(code) = std::env::var("NDM_TEST_INSTALLER_EXIT") {
        std::process::exit(code.parse().unwrap());
    }
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::CreateMutexW};
    let profile = std::env::var_os("NDM_TEST_HANDOFF_PROFILE").expect("fixture profile");
    let profile = std::path::PathBuf::from(profile);
    let name: Vec<u16> = ndm_core::desktop_installation_namespace(&profile)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let guard = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    assert!(!guard.is_null());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !profile.join("parent-released").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "parent waited for the installer to exit instead of completing handoff"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    unsafe {
        CloseHandle(guard);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn windows_handoff_returns_before_installer_exit_and_ignores_own_reservation() {
    let profile = tempfile::tempdir().unwrap();
    let store = ndm_core::Store::open(&profile.path().join("tasks.db")).unwrap();
    let reservation = store.reserve_update_if_idle().unwrap().unwrap();
    assert!(ndm_core::desktop_installation_active(profile.path()).unwrap());
    assert!(!ndm_core::windows_installer_active(profile.path()).unwrap());
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "updates::tests::installer_handoff_fixture",
            "--ignored",
        ])
        .env("NDM_TEST_HANDOFF_PROFILE", profile.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        wait_for_installer_handoff(&mut child, profile.path()),
    )
    .await
    .expect("parent must stop waiting once the installer owns its guard")
    .unwrap();
    assert!(
        child.try_wait().unwrap().is_none(),
        "installer must still be waiting for parent exit"
    );
    drop(reservation);
    assert!(ndm_core::windows_installer_active(profile.path()).unwrap());
    assert!(ndm_core::desktop_installation_active(profile.path()).unwrap());
    std::fs::write(profile.path().join("parent-released"), b"exit").unwrap();
    assert!(child.wait().await.unwrap().success());
    assert!(!ndm_core::desktop_installation_active(profile.path()).unwrap());
}

#[cfg(windows)]
#[tokio::test]
async fn failed_windows_installer_reports_exit_without_quitting_parent() {
    let profile = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "updates::tests::installer_handoff_fixture",
            "--ignored",
        ])
        .env("NDM_TEST_INSTALLER_EXIT", "4")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    assert_eq!(
        wait_for_installer_handoff(&mut child, profile.path()).await,
        Err(UpdateFailure::Installer(Some(4)))
    );
    assert!(!ndm_core::windows_installer_active(profile.path()).unwrap());
}

#[tokio::test]
async fn restricted_update_client_rejects_redirects_before_contacting_untrusted_target() {
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_address = target.local_addr().unwrap();
    let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = source.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = source.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await;
        socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: http://{target_address}/blocked\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let mut release = fixture(b"installer");
    release.url = format!("http://{address}/installer");
    let result = download_with_client(
        release.clone(),
        directory.path().to_owned(),
        client(None).unwrap(),
        4,
        UpdateTransfer::new(release.bytes),
    )
    .await;
    assert_eq!(result, Err(UpdateFailure::Download));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), target.accept())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
