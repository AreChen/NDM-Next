use std::{collections::BTreeMap, sync::Arc, time::Duration};

use ndm_core::{FtpOptions, FtpSecurity, NewTask, TaskId, download_ftp};
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};
use tokio_util::sync::CancellationToken;

struct ResumeServer {
    url: String,
    content: Arc<std::sync::RwLock<Vec<u8>>>,
    offsets: Arc<std::sync::Mutex<Vec<u64>>>,
    attempts: Arc<std::sync::atomic::AtomicUsize>,
    ignore_rest: Arc<std::sync::atomic::AtomicBool>,
    corrupt_transfer: Arc<std::sync::atomic::AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ResumeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn resume_server(content: Vec<u8>, hash: bool, accept_rest: bool) -> ResumeServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let content = Arc::new(std::sync::RwLock::new(content));
    let offsets = Arc::new(std::sync::Mutex::new(Vec::new()));
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ignore_rest = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let corrupt_transfer = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let data = content.clone();
    let positions = offsets.clone();
    let requests = attempts.clone();
    let ignored = ignore_rest.clone();
    let damaged = corrupt_transfer.clone();
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let content = data.clone();
            let offsets = positions.clone();
            let attempts = requests.clone();
            let ignore_rest = ignored.clone();
            let corrupt_transfer = damaged.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let mut reader = BufReader::new(reader).lines();
                if writer.write_all(b"220 Ready\r\n").await.is_err() {
                    return;
                }
                let mut data_listener = None;
                let mut offset = 0;
                while let Ok(Some(line)) = reader.next_line().await {
                    let (command, argument) = line.split_once(' ').unwrap_or((&line, ""));
                    let response = match command {
                        "USER" => "331 Password required\r\n".into(),
                        "PASS" => "230 Logged in\r\n".into(),
                        "TYPE" => "200 Type set\r\n".into(),
                        "OPTS" if hash && argument == "HASH SHA-256" => "200 SHA-256\r\n".into(),
                        "HASH" if hash => {
                            use sha2::{Digest, Sha256};
                            let bytes = content.read().unwrap();
                            format!(
                                "213 SHA-256 0-{} {:x} /file.bin\r\n",
                                bytes.len() - 1,
                                Sha256::digest(&*bytes)
                            )
                        }
                        "SIZE" => format!("213 {}\r\n", content.read().unwrap().len()),
                        "PASV" => {
                            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                            let port = listener.local_addr().unwrap().port();
                            data_listener = Some(listener);
                            format!(
                                "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                                port / 256,
                                port % 256
                            )
                        }
                        "REST" if accept_rest => {
                            offset = if ignore_rest.load(std::sync::atomic::Ordering::SeqCst) {
                                0
                            } else {
                                argument.parse::<usize>().unwrap()
                            };
                            "350 Restart accepted\r\n".into()
                        }
                        "RETR" => {
                            if writer.write_all(b"150 Opening data\r\n").await.is_err() {
                                break;
                            }
                            let (mut stream, _) =
                                data_listener.take().unwrap().accept().await.unwrap();
                            let mut bytes = content.read().unwrap().clone();
                            if corrupt_transfer.load(std::sync::atomic::Ordering::SeqCst) {
                                *bytes.last_mut().unwrap() ^= 1;
                            }
                            offsets.lock().unwrap().push(offset as u64);
                            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            for chunk in bytes[offset..].chunks(4096) {
                                if stream.write_all(chunk).await.is_err() {
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(3)).await;
                            }
                            offset = 0;
                            drop(stream);
                            "226 Transfer complete\r\n".into()
                        }
                        "QUIT" => break,
                        _ => "502 Unsupported\r\n".into(),
                    };
                    if writer.write_all(response.as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    ResumeServer {
        url: format!("ftp://user:correct@{address}/file.bin"),
        content,
        offsets,
        attempts,
        ignore_rest,
        corrupt_transfer,
        task,
    }
}

async fn cancel_after_ftp_partial(dir: &TempDir, server: &ResumeServer, id: TaskId) -> Vec<u8> {
    let partial = dir.path().join(format!(".ndm-next-{id}/ftp.part"));
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let watch = partial.clone();
    tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if tokio::fs::metadata(&watch)
                    .await
                    .is_ok_and(|metadata| metadata.len() >= 128 * 1024)
                {
                    trigger.cancel();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    });
    assert!(matches!(
        download_ftp(
            &request(dir, server.url.clone()),
            id,
            &FtpOptions::default(),
            cancel
        )
        .await,
        Err(ndm_core::Error::Cancelled)
    ));
    assert!(!dir.path().join("file.bin").exists());
    std::fs::read(partial).unwrap()
}

#[tokio::test]
async fn ftp_sha256_resume_uses_persisted_checkpoint_and_rest() {
    // Truncating the existing partial or ignoring the validated REST offset breaks this test.
    let dir = TempDir::new().unwrap();
    let expected: Vec<u8> = (0..512_000).map(|n| (n % 251) as u8).collect();
    let server = resume_server(expected.clone(), true, true).await;
    let id = TaskId::new_v4();
    let partial = cancel_after_ftp_partial(&dir, &server, id).await;
    let checkpoint = std::fs::read(
        dir.path()
            .join(format!(".ndm-next-{id}/protocol-resume.bin")),
    )
    .unwrap();
    assert!(!String::from_utf8_lossy(&checkpoint).contains("correct"));
    let path = download_ftp(
        &request(&dir, server.url.clone()),
        id,
        &FtpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected);
    assert_eq!(*server.offsets.lock().unwrap(), [0, partial.len() as u64]);
}

#[tokio::test]
async fn ftp_without_hash_preserves_partial_while_revalidating_remote_prefix() {
    // Restarting from zero destroys the longer saved prefix during this cancelled validation.
    let dir = TempDir::new().unwrap();
    let expected = vec![29; 512_000];
    let server = resume_server(expected.clone(), false, true).await;
    let id = TaskId::new_v4();
    let partial = cancel_after_ftp_partial(&dir, &server, id).await;
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let attempts = server.attempts.clone();
    tokio::spawn(async move {
        while attempts.load(std::sync::atomic::Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        trigger.cancel();
    });
    assert!(matches!(
        download_ftp(
            &request(&dir, server.url.clone()),
            id,
            &FtpOptions::default(),
            cancel
        )
        .await,
        Err(ndm_core::Error::Cancelled)
    ));
    assert!(
        std::fs::read(dir.path().join(format!(".ndm-next-{id}/ftp.part"))).unwrap() == partial,
        "saved partial was changed during prefix validation"
    );
    let path = download_ftp(
        &request(&dir, server.url.clone()),
        id,
        &FtpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected);
    assert_eq!(*server.offsets.lock().unwrap(), [0, 0, 0]);
}

#[tokio::test]
async fn ftp_changed_resource_and_corrupt_checkpoint_restart_safely() {
    for corrupt in [false, true] {
        let dir = TempDir::new().unwrap();
        let server = resume_server(vec![7; 512_000], true, true).await;
        let id = TaskId::new_v4();
        cancel_after_ftp_partial(&dir, &server, id).await;
        let expected = if corrupt {
            std::fs::write(
                dir.path()
                    .join(format!(".ndm-next-{id}/protocol-resume.bin")),
                b"broken metadata",
            )
            .unwrap();
            vec![7; 512_000]
        } else {
            let bytes = vec![9; 512_000];
            *server.content.write().unwrap() = bytes.clone();
            bytes
        };
        let path = download_ftp(
            &request(&dir, server.url.clone()),
            id,
            &FtpOptions::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), expected);
        assert_eq!(*server.offsets.lock().unwrap(), [0, 0]);
    }
}

#[tokio::test]
async fn ftp_rejected_rest_revalidates_prefix_instead() {
    let dir = TempDir::new().unwrap();
    let expected = vec![17; 512_000];
    let server = resume_server(expected.clone(), true, false).await;
    let id = TaskId::new_v4();
    cancel_after_ftp_partial(&dir, &server, id).await;
    let path = download_ftp(
        &request(&dir, server.url.clone()),
        id,
        &FtpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected);
    assert_eq!(*server.offsets.lock().unwrap(), [0, 0]);
}

#[tokio::test]
async fn ftp_server_that_accepts_but_ignores_rest_cannot_publish_mixed_data() {
    let dir = TempDir::new().unwrap();
    let expected: Vec<u8> = (0..512_000).map(|n| (n % 241) as u8).collect();
    let server = resume_server(expected.clone(), true, true).await;
    let id = TaskId::new_v4();
    cancel_after_ftp_partial(&dir, &server, id).await;
    server
        .ignore_rest
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let result = download_ftp(
        &request(&dir, server.url.clone()),
        id,
        &FtpOptions::default(),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(ndm_core::Error::InvalidFtpResponse)));
    assert!(!dir.path().join("file.bin").exists());
    assert!(
        !dir.path()
            .join(format!(".ndm-next-{id}/protocol-resume.bin"))
            .exists()
    );
    server
        .ignore_rest
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let path = download_ftp(
        &request(&dir, server.url.clone()),
        id,
        &FtpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(std::fs::read(path).unwrap() == expected);
    assert_eq!(server.offsets.lock().unwrap().last(), Some(&0));
}

#[tokio::test]
async fn ftp_same_length_body_must_match_the_full_server_sha256() {
    let dir = TempDir::new().unwrap();
    let server = resume_server(vec![47; 512_000], true, true).await;
    let id = TaskId::new_v4();
    cancel_after_ftp_partial(&dir, &server, id).await;
    server
        .corrupt_transfer
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let result = download_ftp(
        &request(&dir, server.url.clone()),
        id,
        &FtpOptions::default(),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(ndm_core::Error::InvalidFtpResponse)));
    assert!(!dir.path().join("file.bin").exists());
    assert!(
        !dir.path()
            .join(format!(".ndm-next-{id}/protocol-resume.bin"))
            .exists()
    );
}

#[tokio::test]
async fn ftp_corrupt_local_prefix_restarts_but_uncheckpointed_tail_is_discarded() {
    for corrupt in [true, false] {
        let dir = TempDir::new().unwrap();
        let expected = vec![19; 512_000];
        let server = resume_server(expected.clone(), true, true).await;
        let id = TaskId::new_v4();
        let mut saved = cancel_after_ftp_partial(&dir, &server, id).await;
        let saved_length = saved.len();
        if corrupt {
            saved[65_539] = 99;
        } else {
            saved.extend_from_slice(b"uncheckpointed crash tail");
        }
        std::fs::write(dir.path().join(format!(".ndm-next-{id}/ftp.part")), saved).unwrap();
        let path = download_ftp(
            &request(&dir, server.url.clone()),
            id,
            &FtpOptions::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(std::fs::read(path).unwrap() == expected);
        assert_eq!(
            *server.offsets.lock().unwrap(),
            [0, if corrupt { 0 } else { saved_length as u64 }]
        );
    }
}

#[tokio::test]
async fn ftp_without_hash_replaces_changed_same_size_prefix() {
    let dir = TempDir::new().unwrap();
    let server = resume_server(vec![7; 512_000], false, true).await;
    let id = TaskId::new_v4();
    cancel_after_ftp_partial(&dir, &server, id).await;
    let mut expected = vec![7; 512_000];
    expected[65_537] = 53;
    expected[350_001] = 71;
    *server.content.write().unwrap() = expected.clone();
    let path = download_ftp(
        &request(&dir, server.url.clone()),
        id,
        &FtpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(std::fs::read(path).unwrap() == expected);
    assert_eq!(*server.offsets.lock().unwrap(), [0, 0]);
}

async fn ftp_server(content: Vec<u8>, truncate: bool, slow: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let content = Arc::new(content);
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader).lines();
        writer.write_all(b"220 Ready\r\n").await.unwrap();
        let mut data_listener = None;
        while let Ok(Some(line)) = reader.next_line().await {
            let (command, argument) = line.split_once(' ').unwrap_or((&line, ""));
            match command {
                "USER" => writer
                    .write_all(b"331 Password required\r\n")
                    .await
                    .unwrap(),
                "PASS" if argument == "correct" => {
                    writer.write_all(b"230 Logged in\r\n").await.unwrap()
                }
                "PASS" => {
                    writer
                        .write_all(b"530 Authentication failed\r\n")
                        .await
                        .unwrap();
                }
                "TYPE" => writer.write_all(b"200 Type set\r\n").await.unwrap(),
                "SIZE" => writer
                    .write_all(format!("213 {}\r\n", content.len()).as_bytes())
                    .await
                    .unwrap(),
                "PASV" => {
                    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let port = listener.local_addr().unwrap().port();
                    writer
                        .write_all(
                            format!(
                                "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                                port / 256,
                                port % 256
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                    data_listener = Some(listener);
                }
                "RETR" => {
                    writer.write_all(b"150 Opening data\r\n").await.unwrap();
                    let (mut data, _) = data_listener.take().unwrap().accept().await.unwrap();
                    let length = if truncate {
                        content.len() / 2
                    } else {
                        content.len()
                    };
                    for chunk in content[..length].chunks(4096) {
                        if data.write_all(chunk).await.is_err() {
                            break;
                        }
                        if slow {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                    drop(data);
                    if writer
                        .write_all(b"226 Transfer complete\r\n")
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                "QUIT" => {
                    let _ = writer.write_all(b"221 Goodbye\r\n").await;
                    break;
                }
                _ => {
                    let _ = writer.write_all(b"500 Unsupported\r\n").await;
                }
            }
        }
    });
    format!("ftp://user:correct@{address}/file.bin")
}

fn request(dir: &TempDir, url: String) -> NewTask {
    NewTask {
        url,
        destination: dir.path().to_owned(),
        suggested_filename: Some("file.bin".into()),
        source_url: None,
        expected_size: None,
        headers: BTreeMap::new(),
    }
}

#[tokio::test]
async fn ftp_download_verifies_size_and_preserves_existing_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("file.bin"), b"existing").unwrap();
    let bytes = vec![42; 128_000];
    let url = ftp_server(bytes.clone(), false, false).await;
    let output = download_ftp(
        &request(&dir, url),
        TaskId::new_v4(),
        &FtpOptions::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), bytes);
    assert_eq!(
        std::fs::read(dir.path().join("file.bin")).unwrap(),
        b"existing"
    );
}

#[tokio::test]
async fn ftp_authentication_failure_does_not_publish_a_file() {
    let dir = TempDir::new().unwrap();
    let url = ftp_server(vec![1; 64], false, false)
        .await
        .replace(":correct@", ":wrong@");
    assert!(
        download_ftp(
            &request(&dir, url),
            TaskId::new_v4(),
            &FtpOptions::default(),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn truncated_ftp_transfer_is_not_published() {
    let dir = TempDir::new().unwrap();
    let url = ftp_server(vec![7; 64_000], true, false).await;
    assert!(
        download_ftp(
            &request(&dir, url),
            TaskId::new_v4(),
            &FtpOptions::default(),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn cancelled_ftp_transfer_is_not_published() {
    let dir = TempDir::new().unwrap();
    let url = ftp_server(vec![7; 1_000_000], false, true).await;
    let cancel = CancellationToken::new();
    let another = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        another.cancel();
    });
    assert!(matches!(
        download_ftp(
            &request(&dir, url),
            TaskId::new_v4(),
            &FtpOptions::default(),
            cancel
        )
        .await,
        Err(ndm_core::Error::Cancelled)
    ));
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn ftps_urls_cannot_be_downgraded_to_plain_ftp() {
    let dir = TempDir::new().unwrap();
    let req = request(&dir, "ftps://user:password@localhost/file.bin".into());
    let options = FtpOptions {
        security: FtpSecurity::Plain,
        ..Default::default()
    };
    assert!(
        download_ftp(&req, TaskId::new_v4(), &options, CancellationToken::new())
            .await
            .is_err()
    );
}

async fn ftps_server(implicit: bool, content: Vec<u8>) -> (String, Vec<u8>) {
    let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = identity.cert.der().clone();
    let key = PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key.into())
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let content = Arc::new(content);
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let content = content.clone();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if !implicit {
                    if stream.write_all(b"220 Ready\r\n").await.is_err() {
                        return;
                    }
                    let mut command = Vec::new();
                    loop {
                        let mut byte = [0u8; 1];
                        if tokio::io::AsyncReadExt::read_exact(&mut stream, &mut byte)
                            .await
                            .is_err()
                        {
                            return;
                        }
                        command.push(byte[0]);
                        if command.ends_with(b"\r\n") {
                            break;
                        }
                    }
                    assert_eq!(command, b"AUTH TLS\r\n");
                    if stream.write_all(b"234 Begin TLS\r\n").await.is_err() {
                        return;
                    }
                }
                let Ok(stream) = acceptor.accept(stream).await else {
                    return;
                };
                let (reader, mut writer) = tokio::io::split(stream);
                let mut reader = BufReader::new(reader).lines();
                if implicit && writer.write_all(b"220 Ready\r\n").await.is_err() {
                    return;
                }
                let mut data_listener = None;
                while let Ok(Some(line)) = reader.next_line().await {
                    let (command, _) = line.split_once(' ').unwrap_or((&line, ""));
                    let response: &[u8] = match command {
                        "USER" => b"331 Password required\r\n",
                        "PASS" => b"230 Logged in\r\n",
                        "PBSZ" | "PROT" | "TYPE" => b"200 Okay\r\n",
                        "SIZE" => {
                            if writer
                                .write_all(format!("213 {}\r\n", content.len()).as_bytes())
                                .await
                                .is_err()
                            {
                                break;
                            }
                            continue;
                        }
                        "PASV" => {
                            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                            let port = listener.local_addr().unwrap().port();
                            if writer
                                .write_all(
                                    format!(
                                        "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                                        port / 256,
                                        port % 256
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .is_err()
                            {
                                break;
                            }
                            data_listener = Some(listener);
                            continue;
                        }
                        "RETR" => {
                            if writer.write_all(b"150 Opening data\r\n").await.is_err() {
                                break;
                            }
                            let (stream, _) = data_listener.take().unwrap().accept().await.unwrap();
                            let Ok(mut data) = acceptor.accept(stream).await else {
                                break;
                            };
                            for chunk in content.chunks(4096) {
                                if data.write_all(chunk).await.is_err() {
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(2)).await;
                            }
                            let _ = data.shutdown().await;
                            b"226 Transfer complete\r\n"
                        }
                        "QUIT" => {
                            let _ = writer.write_all(b"221 Goodbye\r\n").await;
                            break;
                        }
                        _ => b"500 Unsupported\r\n",
                    };
                    if writer.write_all(response).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (
        format!("ftps://user:correct@localhost:{port}/file.bin"),
        cert.as_ref().to_vec(),
    )
}

#[tokio::test]
async fn ftps_rejects_untrusted_certificates_before_login() {
    let dir = TempDir::new().unwrap();
    let (url, _) = ftps_server(false, vec![1; 16]).await;
    assert!(
        download_ftp(
            &request(&dir, url),
            TaskId::new_v4(),
            &FtpOptions::default(),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn ftps_rejects_trusted_certificate_for_another_host() {
    let dir = TempDir::new().unwrap();
    let (url, cert) = ftps_server(false, vec![1; 16]).await;
    let options = FtpOptions {
        additional_ca_der: Some(cert),
        ..Default::default()
    };
    let wrong_host = url.replace("localhost", "127.0.0.1");
    assert!(
        download_ftp(
            &request(&dir, wrong_host),
            TaskId::new_v4(),
            &options,
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn explicit_and_implicit_ftps_verify_tls_and_download() {
    for implicit in [false, true] {
        let dir = TempDir::new().unwrap();
        let expected = vec![23; 32_768];
        let (url, cert) = ftps_server(implicit, expected.clone()).await;
        let options = FtpOptions {
            security: if implicit {
                FtpSecurity::ImplicitTls
            } else {
                FtpSecurity::ExplicitTls
            },
            additional_ca_der: Some(cert),
            ..Default::default()
        };
        let path = download_ftp(
            &request(&dir, url),
            TaskId::new_v4(),
            &options,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), expected);
    }
}

#[tokio::test]
async fn explicit_and_implicit_ftps_continue_protected_partials_after_cancel() {
    for implicit in [false, true] {
        let dir = TempDir::new().unwrap();
        let expected: Vec<u8> = (0..512_000).map(|n| (n % 227) as u8).collect();
        let (url, cert) = ftps_server(implicit, expected.clone()).await;
        let options = FtpOptions {
            security: if implicit {
                FtpSecurity::ImplicitTls
            } else {
                FtpSecurity::ExplicitTls
            },
            additional_ca_der: Some(cert),
            ..Default::default()
        };
        let id = TaskId::new_v4();
        let temporary = dir.path().join(format!(".ndm-next-{id}/ftp.part"));
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let watch = temporary.clone();
        tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if tokio::fs::metadata(&watch)
                        .await
                        .is_ok_and(|m| m.len() >= 128 * 1024)
                    {
                        trigger.cancel();
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            })
            .await
            .unwrap();
        });
        assert!(matches!(
            download_ftp(&request(&dir, url.clone()), id, &options, cancel).await,
            Err(ndm_core::Error::Cancelled)
        ));
        let saved = std::fs::read(&temporary).unwrap();
        assert!(!saved.is_empty());
        let checkpoint = std::fs::read(
            dir.path()
                .join(format!(".ndm-next-{id}/protocol-resume.bin")),
        )
        .unwrap();
        #[cfg(windows)]
        assert!(ndm_core::unprotect_user_data(&checkpoint).is_ok());
        assert!(!String::from_utf8_lossy(&checkpoint).contains("correct"));
        let path = download_ftp(&request(&dir, url), id, &options, CancellationToken::new())
            .await
            .unwrap();
        assert!(std::fs::read(path).unwrap() == expected);
        assert!(!temporary.exists());
    }
}
