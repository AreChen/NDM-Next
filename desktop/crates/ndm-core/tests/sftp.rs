use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

use ndm_core::{NewTask, SftpAuth, SftpOptions, TaskId, download_sftp};
use russh::{
    Channel, ChannelId,
    keys::{Algorithm, HashAlg, PrivateKey, PublicKey},
    server::{Auth, ChannelOpenHandle, Msg, Session, VerifiedPublicKey},
};
use russh_sftp::protocol::{Attrs, Data, FileAttributes, Handle, OpenFlags, Status, StatusCode};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

struct SshFixture {
    channel: Option<Channel<Msg>>,
    data: Arc<Vec<u8>>,
    accepted_key: Option<PublicKey>,
    slow: bool,
    truncate: bool,
}

impl russh::server::Handler for SshFixture {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "user" && password == "correct" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(
            if user == "user" && self.accepted_key.as_ref() == Some(key.public_key) {
                Auth::Accept
            } else {
                Auth::reject()
            },
        )
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channel = Some(channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            session.channel_failure(channel_id)?;
            return Ok(());
        }
        let channel = self.channel.take().unwrap();
        let fixture = FileFixture {
            data: self.data.clone(),
            slow: self.slow,
            truncate: self.truncate,
        };
        session.channel_success(channel_id)?;
        tokio::spawn(async move {
            russh_sftp::server::run(channel.into_stream(), fixture).await;
        });
        Ok(())
    }
}

struct FileFixture {
    data: Arc<Vec<u8>>,
    slow: bool,
    truncate: bool,
}

impl russh_sftp::server::Handler for FileFixture {
    type Error = StatusCode;
    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn open(
        &mut self,
        id: u32,
        _: String,
        _: OpenFlags,
        _: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        Ok(Handle {
            id,
            handle: "file".into(),
        })
    }

    async fn stat(&mut self, id: u32, _: String) -> Result<Attrs, Self::Error> {
        Ok(Attrs {
            id,
            attrs: FileAttributes {
                size: Some(self.data.len() as u64),
                ..Default::default()
            },
        })
    }

    async fn fstat(&mut self, id: u32, _: String) -> Result<Attrs, Self::Error> {
        self.stat(id, String::new()).await
    }

    async fn read(
        &mut self,
        id: u32,
        _: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        if self.slow {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let end = if self.truncate {
            self.data.len() / 2
        } else {
            self.data.len()
        };
        let start = offset as usize;
        if start >= end {
            return Err(StatusCode::Eof);
        }
        let stop = (start + len as usize).min(end);
        Ok(Data {
            id,
            data: self.data[start..stop].to_vec(),
        })
    }

    async fn close(&mut self, id: u32, _: String) -> Result<Status, Self::Error> {
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: String::new(),
            language_tag: String::new(),
        })
    }
}

async fn server(
    data: Vec<u8>,
    accepted_key: Option<PublicKey>,
    slow: bool,
    truncate: bool,
) -> (String, String) {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let config = Arc::new(russh::server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let fixture = SshFixture {
            channel: None,
            data: Arc::new(data),
            accepted_key,
            slow,
            truncate,
        };
        if let Ok(session) = russh::server::run_stream(config, socket, fixture).await {
            let _ = session.await;
        }
    });
    (format!("sftp://user:correct@{addr}/file.bin"), fingerprint)
}

fn request(dir: &Path, url: String) -> NewTask {
    NewTask {
        url,
        destination: dir.to_owned(),
        suggested_filename: Some("file.bin".into()),
        source_url: None,
        expected_size: None,
        headers: BTreeMap::new(),
    }
}

#[tokio::test]
async fn sftp_unknown_host_is_rejected_and_returns_fingerprint() {
    let dir = TempDir::new().unwrap();
    let (url, fingerprint) = server(vec![1; 100], None, false, false).await;
    let result = download_sftp(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &SftpOptions::default(),
        CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(result, Err(ndm_core::Error::SftpUnknownHost { fingerprint: observed }) if observed == fingerprint)
    );
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn sftp_changed_host_key_is_rejected() {
    let dir = TempDir::new().unwrap();
    let (url, fingerprint) = server(vec![1; 100], None, false, false).await;
    let options = SftpOptions {
        trusted_fingerprint: Some("SHA256:wrong".into()),
        ..Default::default()
    };
    let result = download_sftp(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &options,
        CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(result, Err(ndm_core::Error::SftpHostKeyChanged { observed, .. }) if observed == fingerprint)
    );
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn sftp_rejects_wrong_password_without_publishing() {
    let dir = TempDir::new().unwrap();
    let (url, fingerprint) = server(vec![1; 100], None, false, false).await;
    let url = url.replace(":correct@", ":wrong@");
    let options = SftpOptions {
        trusted_fingerprint: Some(fingerprint),
        ..Default::default()
    };
    let result = download_sftp(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &options,
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(ndm_core::Error::SftpAuthentication)));
    assert!(!dir.path().join("file.bin").exists());
}

#[tokio::test]
async fn sftp_password_download_and_truncation_are_checked() {
    for truncate in [false, true] {
        let dir = TempDir::new().unwrap();
        let bytes = vec![39; 128_000];
        let (url, fingerprint) = server(bytes.clone(), None, false, truncate).await;
        let options = SftpOptions {
            trusted_fingerprint: Some(fingerprint),
            ..Default::default()
        };
        let result = download_sftp(
            &request(dir.path(), url),
            TaskId::new_v4(),
            &options,
            CancellationToken::new(),
        )
        .await;
        if truncate {
            assert!(result.is_err());
            assert!(!dir.path().join("file.bin").exists());
        } else {
            assert_eq!(std::fs::read(result.unwrap()).unwrap(), bytes);
        }
    }
}

#[tokio::test]
async fn sftp_supports_key_authentication() {
    let dir = TempDir::new().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key_path = dir.path().join("id_ed25519");
    std::fs::write(
        &key_path,
        key.to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let bytes = vec![42; 10_000];
    let (url, fingerprint) =
        server(bytes.clone(), Some(key.public_key().clone()), false, false).await;
    let options = SftpOptions {
        trusted_fingerprint: Some(fingerprint),
        auth: Some(SftpAuth::PrivateKey {
            path: key_path,
            passphrase: None,
        }),
        ..Default::default()
    };
    let path = download_sftp(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &options,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[tokio::test]
async fn sftp_cancel_does_not_publish_partial_output() {
    let dir = TempDir::new().unwrap();
    let (url, fingerprint) = server(vec![3; 1_000_000], None, true, false).await;
    let options = SftpOptions {
        trusted_fingerprint: Some(fingerprint),
        ..Default::default()
    };
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(60)).await;
        trigger.cancel();
    });
    let result = download_sftp(
        &request(dir.path(), url),
        TaskId::new_v4(),
        &options,
        cancel,
    )
    .await;
    assert!(matches!(result, Err(ndm_core::Error::Cancelled)));
    assert!(!dir.path().join("file.bin").exists());
}
