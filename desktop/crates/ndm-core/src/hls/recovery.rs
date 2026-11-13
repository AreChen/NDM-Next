//! Persist the engine's committed-prefix checkpoints in a small SQLite database.
use std::path::Path;

use hls_engine::EngineCheckpoint;
use rusqlite::{Connection, OptionalExtension, params};

use crate::{Error, Result};

pub(super) fn load(path: &Path) -> Result<Option<EngineCheckpoint>> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(Error::HlsRecovery);
        }
        Ok(_) => {}
    }
    let db = Connection::open(path)?;
    let bytes: Option<Vec<u8>> = db
        .query_row(
            "SELECT data FROM checkpoint WHERE id = 1 AND length(data) <= 67108864",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let bytes = bytes.ok_or(Error::HlsRecovery)?;
    Ok(Some(
        EngineCheckpoint::from_bytes(&bytes).map_err(|_| Error::HlsRecovery)?,
    ))
}

pub(super) fn save(
    path: &Path,
    checkpoint: &EngineCheckpoint,
) -> std::result::Result<(), rusqlite::Error> {
    let mut db = Connection::open(path)?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS checkpoint (id INTEGER PRIMARY KEY CHECK (id = 1), data BLOB NOT NULL)")?;
    tx.execute(
        "INSERT OR REPLACE INTO checkpoint (id, data) VALUES (1, ?1)",
        params![checkpoint.to_bytes()],
    )?;
    tx.commit()
}

pub(super) async fn remove_files(work_dir: &Path) {
    for name in [
        "hls-output.mp4",
        "hls-output.mp4.hls-partial",
        "hls-checkpoint.sqlite",
        "hls-checkpoint.sqlite-journal",
    ] {
        let _ = tokio::fs::remove_file(work_dir.join(name)).await;
    }
}
