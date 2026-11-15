use super::*;
use serde_json::{Value, json};

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
async fn response(body: &'static [u8], advertised: usize) -> reqwest::Response {
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
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{address}/installer"))
        .send()
        .await
        .unwrap()
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
    let ready = download(release, directory.path().to_owned(), proxy)
        .await
        .unwrap();
    verify_cached(&ready).await.unwrap();
    println!(
        "Verified desktop {}: {} bytes, {:x}",
        ready.release.version,
        ready.release.bytes,
        Sha256::digest(std::fs::read(&ready.path).unwrap())
    );
}
