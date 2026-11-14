//! Persistent continuation for protocols without HTTP-style immutable validators.
//!
//! A local digest detects damaged/crash-incomplete partials. Unless a server supplies
//! a full-file SHA-256, every retained byte is read again and compared with the same
//! remote stream that supplies the suffix. Size, timestamps and SSH host identity
//! alone never authorize skipping content validation.

use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

const MAX_CHECKPOINT_BYTES: u64 = 16 * 1024;
const CHECKPOINT_INTERVAL: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u32,
    source: [u8; 32],
    size: Option<u64>,
    length: u64,
    sha256: [u8; 32],
    remote_sha256: Option<[u8; 32]>,
}

pub(crate) struct PartialFile {
    output: fs::File,
    comparison: fs::File,
    checkpoint_path: PathBuf,
    checkpoint: Checkpoint,
    /// Digest of the bytes received on this connection, including a proved REST prefix.
    stream_hash: Sha256,
    received: u64,
    length: u64,
    preserved: u64,
    remote_sha256: Option<[u8; 32]>,
    can_resume: bool,
    dirty: bool,
}

pub(crate) fn source_binding(value: &impl Serialize) -> Result<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(value)?).into())
}

async fn read_checkpoint(path: &Path) -> Option<Checkpoint> {
    let file = fs::File::open(path).await.ok()?;
    if file.metadata().await.ok()?.len() > MAX_CHECKPOINT_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_CHECKPOINT_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
        return None;
    }
    #[cfg(windows)]
    let bytes = crate::secrets::unprotect(&bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

impl PartialFile {
    pub(crate) async fn open(
        work_dir: &Path,
        temporary: &Path,
        source: [u8; 32],
        size: Option<u64>,
        remote_sha256: Option<[u8; 32]>,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let checkpoint_path = work_dir.join("protocol-resume.bin");
        let mut output = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(temporary)
            .await?;
        let file_length = output.metadata().await?.len();
        let saved = read_checkpoint(&checkpoint_path).await.filter(|saved| {
            saved.version == 1 && saved.source == source
                && saved.length <= file_length
                && size.is_none_or(|size| saved.length <= size)
                && !matches!((saved.size, size), (Some(old), Some(new)) if old != new)
                && !matches!((saved.remote_sha256, remote_sha256), (Some(old), Some(new)) if old != new)
        });
        let mut local_hash = Sha256::new();
        let saved = if let Some(saved) = saved {
            let mut remaining = saved.length;
            let mut buffer = [0u8; 64 * 1024];
            while remaining > 0 {
                let take = (remaining as usize).min(buffer.len());
                let count = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(Error::Cancelled),
                    result = output.read(&mut buffer[..take]) => result?,
                };
                if count == 0 {
                    break;
                }
                local_hash.update(&buffer[..count]);
                remaining -= count as u64;
            }
            (remaining == 0 && <[u8; 32]>::from(local_hash.clone().finalize()) == saved.sha256)
                .then_some(saved)
        } else {
            None
        };
        let checkpoint = saved.unwrap_or_else(|| Checkpoint {
            version: 1,
            source,
            size,
            length: 0,
            sha256: Sha256::digest([]).into(),
            remote_sha256,
        });
        let length = checkpoint.length;
        // Ignore any bytes written after the last durable checkpoint, including crash tails.
        output.set_len(length).await?;
        output.seek(SeekFrom::Start(length)).await?;
        let can_resume =
            length > 0 && remote_sha256.is_some() && checkpoint.remote_sha256 == remote_sha256;
        let stream_hash = if can_resume {
            local_hash
        } else {
            Sha256::new()
        };
        let comparison = fs::File::open(temporary).await?;
        let mut partial = Self {
            output,
            comparison,
            checkpoint_path,
            checkpoint,
            stream_hash,
            received: if can_resume { length } else { 0 },
            length,
            preserved: length,
            remote_sha256,
            can_resume,
            dirty: false,
        };
        partial.checkpoint.size = size;
        // Publish the initial empty state too, so a later abrupt shutdown has an honest boundary.
        partial.save_checkpoint().await?;
        Ok(partial)
    }

    pub(crate) fn resume_offset(&self) -> u64 {
        if self.can_resume { self.length } else { 0 }
    }

    pub(crate) fn received(&self) -> u64 {
        self.received
    }

    /// A server that refuses REST can still validate and reuse the saved prefix.
    pub(crate) fn require_prefix_validation(&mut self) {
        self.received = 0;
        self.stream_hash = Sha256::new();
        self.can_resume = false;
    }

    pub(crate) async fn consume(&mut self, bytes: &[u8]) -> Result<()> {
        let compare = self
            .preserved
            .saturating_sub(self.received)
            .min(bytes.len() as u64) as usize;
        let mut skip = 0;
        if compare > 0 {
            // Callers pass at most 64 KiB; never buffer an entire remote prefix.
            let mut previous = vec![0u8; compare];
            self.comparison.read_exact(&mut previous).await?;
            if previous == bytes[..compare] {
                skip = compare;
            } else {
                // Earlier retained bytes were compared byte-for-byte on this same stream.
                // Replace the first mismatching block and every following byte with new data.
                self.output.set_len(self.received).await?;
                self.output.seek(SeekFrom::Start(self.received)).await?;
                self.length = self.received;
                self.preserved = self.received;
                self.dirty = true;
            }
        }
        if skip < bytes.len() {
            self.output.write_all(&bytes[skip..]).await?;
            self.length = self
                .length
                .checked_add((bytes.len() - skip) as u64)
                .ok_or_else(|| std::io::Error::other("download length overflow"))?;
            self.dirty = true;
        }
        self.stream_hash.update(bytes);
        self.received = self
            .received
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("download length overflow"))?;
        if self.dirty && self.length.saturating_sub(self.checkpoint.length) >= CHECKPOINT_INTERVAL {
            self.save_checkpoint().await?;
        }
        Ok(())
    }

    /// Called after a successful EOF. An unknown-size remote may have become shorter.
    pub(crate) async fn finish(&mut self) -> Result<()> {
        if self.received < self.length {
            self.output.set_len(self.received).await?;
            self.length = self.received;
            self.dirty = true;
        }
        self.save_checkpoint().await
    }

    pub(crate) fn matches_remote_hash(&self) -> bool {
        self.remote_sha256.is_none_or(|expected| {
            self.received == self.length
                && <[u8; 32]>::from(self.stream_hash.clone().finalize()) == expected
        })
    }

    /// Persist on cancellation and errors, while keeping a not-yet-validated old prefix intact.
    pub(crate) async fn save_checkpoint(&mut self) -> Result<()> {
        self.output.flush().await?;
        self.output.sync_all().await?;
        if self.dirty {
            self.checkpoint.length = self.length;
            self.checkpoint.sha256 = self.stream_hash.clone().finalize().into();
            // Only associate a newly learned remote hash after all retained bytes were checked.
            self.checkpoint.remote_sha256 = self.remote_sha256;
        }
        let bytes = serde_json::to_vec(&self.checkpoint)?;
        #[cfg(windows)]
        let bytes = crate::secrets::protect(&bytes)?;
        // On other platforms only opaque source/digest fingerprints are serialized, never URLs.
        let pending = self.checkpoint_path.with_extension("pending");
        let mut file = fs::File::create(&pending).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        fs::rename(&pending, &self.checkpoint_path).await?;
        self.dirty = false;
        Ok(())
    }

    pub(crate) async fn invalidate(&self) -> Result<()> {
        match fs::remove_file(&self.checkpoint_path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) async fn cleanup(work_dir: &Path) {
        let _ = fs::remove_file(work_dir.join("protocol-resume.bin")).await;
        let _ = fs::remove_file(work_dir.join("protocol-resume.pending")).await;
        let _ = fs::remove_dir(work_dir).await;
    }
}
