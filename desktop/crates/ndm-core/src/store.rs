use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{Error, NewTask, Result, Task, TaskId, TaskState, secrets};

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let store = Self {
            path: path.to_owned(),
        };
        let mut conn = store.connection()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 1 {
            return Err(Error::UnsupportedSchema);
        }
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY NOT NULL,
                request BLOB NOT NULL,
                state TEXT NOT NULL CHECK(state IN
                    ('prepared','queued','running','paused','completed','failed')),
                output BLOB,
                error TEXT,
                size_verified INTEGER,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );",
        )?;
        let columns: Vec<String> = {
            let mut statement = tx.prepare("PRAGMA table_info(tasks)")?;
            statement
                .query_map([], |row| row.get(1))?
                .collect::<std::result::Result<_, _>>()?
        };
        for (name, declaration) in [
            ("output", "BLOB"),
            ("error", "TEXT"),
            ("size_verified", "INTEGER"),
        ] {
            if !columns.iter().any(|column| column == name) {
                tx.execute(
                    &format!("ALTER TABLE tasks ADD COLUMN {name} {declaration}"),
                    [],
                )?;
            }
        }
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        Ok(conn)
    }

    pub fn prepare(&self, id: TaskId, request: &NewTask) -> Result<Task> {
        let encrypted = secrets::protect(&serde_json::to_vec(request)?)?;
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = read_task(&tx, id)? {
            if existing.request != *request {
                return Err(Error::IdConflict);
            }
            tx.commit()?;
            return Ok(existing);
        }
        tx.execute(
            "INSERT INTO tasks (id, request, state) VALUES (?1, ?2, 'prepared')",
            params![id.to_string(), encrypted],
        )?;
        let task = read_task(&tx, id)?.ok_or(Error::NotFound)?;
        tx.commit()?;
        Ok(task)
    }

    pub fn commit(&self, id: TaskId) -> Result<Task> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE tasks SET state = 'queued' WHERE id = ?1 AND state = 'prepared'",
            [id.to_string()],
        )?;
        let task = read_task(&tx, id)?.ok_or(Error::NotFound)?;
        tx.commit()?;
        Ok(task)
    }

    pub fn abort(&self, id: TaskId) -> Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = read_task(&tx, id)?;
        match task {
            None => {}
            Some(task) if task.state == TaskState::Prepared => {
                tx.execute("DELETE FROM tasks WHERE id = ?1", [id.to_string()])?;
            }
            Some(_) => return Err(Error::InvalidState),
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get(&self, id: TaskId) -> Result<Option<Task>> {
        read_task(&self.connection()?, id)
    }

    pub fn list(&self) -> Result<Vec<Task>> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT id, request, state, output, error, size_verified FROM tasks ORDER BY rowid",
        )?;
        let mut rows = statement.query([])?;
        let mut tasks = Vec::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let id = TaskId::parse_str(&id).map_err(|_| Error::InvalidStoredState)?;
            let encrypted: Vec<u8> = row.get(1)?;
            let state: String = row.get(2)?;
            let output: Option<Vec<u8>> = row.get(3)?;
            let error: Option<String> = row.get(4)?;
            let size_verified: Option<bool> = row.get(5)?;
            tasks.push(decode_task(
                id,
                &encrypted,
                &state,
                output,
                error,
                size_verified,
            )?);
        }
        Ok(tasks)
    }

    pub fn start(&self, id: TaskId) -> Result<()> {
        self.transition(id, "queued", "running")
    }

    pub fn pause(&self, id: TaskId) -> Result<()> {
        let conn = self.connection()?;
        let changed = conn.execute(
            "UPDATE tasks SET state = 'paused' WHERE id = ?1 AND state IN ('queued','running')",
            [id.to_string()],
        )?;
        if changed == 1 || self.get(id)?.is_some_and(|t| t.state == TaskState::Paused) {
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }

    fn transition(&self, id: TaskId, from: &str, to: &str) -> Result<()> {
        let conn = self.connection()?;
        let changed = conn.execute(
            "UPDATE tasks SET state = ?1 WHERE id = ?2 AND state = ?3",
            params![to, id.to_string(), from],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }

    pub fn recover_interrupted(&self) -> Result<()> {
        self.connection()?.execute(
            "UPDATE tasks SET state = 'queued' WHERE state = 'running'",
            [],
        )?;
        Ok(())
    }

    pub fn resume(&self, id: TaskId) -> Result<()> {
        let changed = self.connection()?.execute(
            "UPDATE tasks SET state = 'queued', error = NULL WHERE id = ?1 AND state IN ('paused','failed')",
            [id.to_string()],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }

    pub(crate) fn complete(&self, id: TaskId, path: &Path, size_verified: bool) -> Result<()> {
        let output = secrets::protect(&serde_json::to_vec(path)?)?;
        let changed = self.connection()?.execute(
            "UPDATE tasks SET state = 'completed', output = ?1, size_verified = ?2, error = NULL WHERE id = ?3 AND state = 'running'",
            params![output, size_verified, id.to_string()],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }

    pub(crate) fn fail(&self, id: TaskId, reason: &'static str) -> Result<()> {
        let changed = self.connection()?.execute(
            "UPDATE tasks SET state = 'failed', error = ?1 WHERE id = ?2 AND state = 'running'",
            params![reason, id.to_string()],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }
}

fn read_task(conn: &Connection, id: TaskId) -> Result<Option<Task>> {
    let row = conn
        .query_row(
            "SELECT request, state, output, error, size_verified FROM tasks WHERE id = ?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<bool>>(4)?,
                ))
            },
        )
        .optional()?;
    row.map(|(encrypted, state, output, error, size_verified)| {
        decode_task(id, &encrypted, &state, output, error, size_verified)
    })
    .transpose()
}

fn decode_task(
    id: TaskId,
    encrypted: &[u8],
    state: &str,
    output: Option<Vec<u8>>,
    error: Option<String>,
    size_verified: Option<bool>,
) -> Result<Task> {
    let request = serde_json::from_slice(&secrets::unprotect(encrypted)?)?;
    let output_path = output
        .map(|value| {
            let plaintext = secrets::unprotect(&value)?;
            Ok::<_, Error>(serde_json::from_slice(&plaintext)?)
        })
        .transpose()?;
    Ok(Task {
        id,
        request,
        state: TaskState::from_database(state)?,
        output_path,
        error,
        size_verified,
    })
}
