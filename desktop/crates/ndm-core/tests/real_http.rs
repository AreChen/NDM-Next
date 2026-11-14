//! Opt-in public HTTP smoke test. Ordinary `cargo test` never runs this test.
//!
//! Set NDM_REAL_HTTP_URL to a nonempty, disposable HTTP(S) download fixture, then
//! run `rtk proxy cargo test -p ndm-core --test real_http -- --ignored --nocapture`.
//! Optional variables:
//! - NDM_REAL_HTTP_PROXY: HTTP(S) proxy URL without embedded credentials.
//! - NDM_REAL_HTTP_EXPECT_UNKNOWN_SIZE=1: require the probe to omit both length
//!   headers, so a known-size response cannot accidentally pass that scenario.
//! - NDM_REAL_HTTP_EXPECT_BYTES: independently known complete file length.
//! - NDM_REAL_HTTP_EXPECT_SHA256: independently known 64-digit SHA-256 digest.
//! - NDM_REAL_HTTP_KEEP_OUTPUT=1: retain the temporary output for an external
//!   archive/CRC check. Otherwise the file is removed when the test finishes.
//!
//! The probe is a separate GET, so use reusable URLs. A live branch archive can
//! change between requests; pin a revision when comparing an expected digest.

use std::{
    collections::BTreeMap,
    env,
    fs::File,
    io::Read,
    time::{Duration, Instant},
};

use ndm_core::{HttpOptions, NewTask, ProxyOptions, TaskId, download_http};
use reqwest::{Url, header};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn enabled(name: &str) -> bool {
    env::var(name).is_ok_and(|value| value == "1")
}

// URLs can contain credentials, signed query strings or fragments. The fixture
// location is useful evidence, but those components do not belong in test logs.
fn public_location(url: &Url) -> String {
    let mut location = url.clone();
    let _ = location.set_username("");
    let _ = location.set_password(None);
    location.set_query(None);
    location.set_fragment(None);
    location.into()
}

#[tokio::test]
#[ignore = "requires explicit NDM_REAL_HTTP_URL and real network access"]
async fn real_http_download_follows_redirects_and_publishes_complete_output() {
    let url = env::var("NDM_REAL_HTTP_URL")
        .expect("set NDM_REAL_HTTP_URL explicitly before running this ignored test");
    let parsed = Url::parse(&url).expect("NDM_REAL_HTTP_URL must be an absolute URL");
    assert!(matches!(parsed.scheme(), "http" | "https"));
    assert!(parsed.username().is_empty() && parsed.password().is_none());

    let expected_sha256 = env::var("NDM_REAL_HTTP_EXPECT_SHA256").ok();
    if let Some(expected) = &expected_sha256 {
        assert!(expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    let expected_bytes = env::var("NDM_REAL_HTTP_EXPECT_BYTES").ok().map(|value| {
        value
            .parse::<u64>()
            .expect("NDM_REAL_HTTP_EXPECT_BYTES must be an unsigned integer")
    });
    let proxy = env::var("NDM_REAL_HTTP_PROXY")
        .ok()
        .map(|url| ProxyOptions {
            url,
            username: None,
            password: None,
        });
    if let Some(proxy) = &proxy {
        proxy.validate().expect("invalid NDM_REAL_HTTP_PROXY");
    }

    let mut client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::limited(10))
        .referer(false);
    if let Some(proxy) = &proxy {
        client = client.proxy(reqwest::Proxy::all(&proxy.url).unwrap());
    }
    let probe_started = Instant::now();
    let response = client
        .build()
        .unwrap()
        .get(parsed.clone())
        .header(header::RANGE, "bytes=0-0")
        .send()
        .await
        .expect("real HTTP probe failed");
    assert!(
        response.status().is_success(),
        "probe returned {}",
        response.status()
    );
    println!("requested_url={}", public_location(&parsed));
    println!("effective_url={}", public_location(response.url()));
    println!(
        "probe_status={} probe_elapsed_ms={} effective_scheme={}",
        response.status().as_u16(),
        probe_started.elapsed().as_millis(),
        response.url().scheme(),
    );
    for name in [
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::TRANSFER_ENCODING,
        header::CONTENT_TYPE,
        header::CONTENT_DISPOSITION,
        header::ETAG,
        header::ACCEPT_RANGES,
    ] {
        println!("{}={:?}", name, response.headers().get(&name));
    }
    let advertised_full_size = (response.status() == reqwest::StatusCode::OK)
        .then(|| response.content_length())
        .flatten();
    if enabled("NDM_REAL_HTTP_EXPECT_UNKNOWN_SIZE") {
        assert!(
            !response.headers().contains_key(header::CONTENT_LENGTH)
                && !response.headers().contains_key(header::CONTENT_RANGE),
            "fixture must actually omit complete-length metadata",
        );
        assert!(advertised_full_size.is_none());
    }
    drop(response);

    let directory = TempDir::new().unwrap();
    let request = NewTask {
        url,
        destination: directory.path().to_path_buf(),
        suggested_filename: None,
        source_url: None,
        // Deliberately leave the caller's length unknown, including when the
        // optional independent length assertion is supplied for verification.
        expected_size: None,
        headers: BTreeMap::new(),
    };
    let options = HttpOptions {
        proxy,
        ..HttpOptions::default()
    };
    let started = Instant::now();
    let output = tokio::time::timeout(
        Duration::from_secs(180),
        download_http(
            &request,
            TaskId::new_v4(),
            &options,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("real HTTP download exceeded 180 seconds")
    .expect("real core HTTP download failed");
    let download_elapsed = started.elapsed();

    let canonical_directory = directory.path().canonicalize().unwrap();
    let canonical_output = output.canonicalize().unwrap();
    assert_eq!(
        canonical_output.parent(),
        Some(canonical_directory.as_path())
    );
    let metadata = std::fs::symlink_metadata(&output).unwrap();
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    assert!(metadata.len() > 0, "use a nonempty smoke fixture");
    let filename = output.file_name().and_then(|name| name.to_str()).unwrap();
    assert!(!filename.is_empty() && !filename.chars().any(char::is_control));
    assert!(!filename.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*']));
    assert!(!filename.ends_with([' ', '.']));

    let mut file = File::open(&output).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut counted = 0_u64;
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        counted += count as u64;
    }
    assert_eq!(counted, metadata.len());
    if let Some(expected) = expected_bytes.or(advertised_full_size) {
        assert_eq!(counted, expected, "complete downloaded length mismatch");
    }
    let actual_sha256 = format!("{:x}", digest.finalize());
    if let Some(expected) = expected_sha256 {
        assert!(
            actual_sha256.eq_ignore_ascii_case(&expected),
            "SHA-256 mismatch"
        );
    }
    println!(
        "filename={filename:?} bytes={counted} sha256={actual_sha256} download_elapsed_ms={}",
        download_elapsed.as_millis(),
    );
    if enabled("NDM_REAL_HTTP_KEEP_OUTPUT") {
        let kept = directory.keep();
        println!(
            "kept_directory={} output={}",
            kept.display(),
            output.display()
        );
    }
}
