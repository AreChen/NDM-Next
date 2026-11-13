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
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
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
                    if data.write_all(&content).await.is_err() {
                        break;
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
