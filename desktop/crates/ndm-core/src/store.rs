use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{Error, HlsOptions, NewTask, Result, Task, TaskId, TaskOptions, TaskState, secrets};

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
        if version > 2 {
            return Err(Error::UnsupportedSchema);
        }
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY NOT NULL,
                request BLOB NOT NULL,
                options BLOB,
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
            ("options", "BLOB"),
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
        tx.pragma_update(None, "user_version", 2)?;
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
        self.prepare_with_options(id, request, &TaskOptions::Auto)
    }

    pub fn prepare_with_options(
        &self,
        id: TaskId,
        request: &NewTask,
        options: &TaskOptions,
    ) -> Result<Task> {
        let encrypted = secrets::protect(&serde_json::to_vec(request)?)?;
        let encrypted_options = secrets::protect(&serde_json::to_vec(options)?)?;
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = read_task(&tx, id)? {
            if existing.request != *request
                || (existing.state == TaskState::Prepared && existing.options != *options)
            {
                return Err(Error::IdConflict);
            }
            tx.commit()?;
            return Ok(existing);
        }
        tx.execute(
            "INSERT INTO tasks (id, request, options, state) VALUES (?1, ?2, ?3, 'prepared')",
            params![id.to_string(), encrypted, encrypted_options],
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
            "SELECT id, request, state, output, error, size_verified, options FROM tasks ORDER BY rowid",
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
            let options: Option<Vec<u8>> = row.get(6)?;
            tasks.push(decode_task(
                id,
                &encrypted,
                &state,
                output,
                error,
                size_verified,
                options,
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

    pub fn recover_without_autostart(&self) -> Result<()> {
        self.connection()?.execute(
            "UPDATE tasks SET state = 'paused' WHERE state IN ('running', 'queued')",
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

    /// Replace an expired web address while no worker can be using the task.
    /// Credentials from the old origin must never be forwarded to another one.
    pub fn update_web_url(&self, id: TaskId, url: &str) -> Result<()> {
        fn web_url(url: &str) -> Result<reqwest::Url> {
            let parsed = reqwest::Url::parse(url).map_err(|_| Error::InvalidDownloadAddress)?;
            if url.len() > 65_536
                || !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(Error::InvalidDownloadAddress);
            }
            Ok(parsed)
        }
        let replacement = web_url(url)?;
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = read_task(&tx, id)?.ok_or(Error::NotFound)?;
        if !matches!(task.state, TaskState::Paused | TaskState::Failed)
            || matches!(task.options, TaskOptions::Ftp(_) | TaskOptions::Sftp(_))
        {
            return Err(Error::InvalidState);
        }
        let previous = web_url(&task.request.url)?;
        let options = match task.options {
            TaskOptions::Auto if previous.path().to_ascii_lowercase().ends_with(".m3u8") => {
                TaskOptions::Hls(HlsOptions::default())
            }
            TaskOptions::Auto => TaskOptions::AutoHttp,
            other => other,
        };
        let mut request = task.request;
        if previous.origin() != replacement.origin() {
            request.headers.clear();
            request.source_url = None;
        }
        request.url = replacement.to_string();
        let encrypted = secrets::protect(&serde_json::to_vec(&request)?)?;
        let encrypted_options = secrets::protect(&serde_json::to_vec(&options)?)?;
        tx.execute(
            "UPDATE tasks SET request = ?1, options = ?2, error = NULL WHERE id = ?3",
            params![encrypted, encrypted_options, id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes only the database record; downloaded and temporary files are preserved.
    pub fn delete(&self, id: TaskId) -> Result<()> {
        let changed = self.connection()?.execute(
            "DELETE FROM tasks WHERE id = ?1 AND state IN ('queued','paused','completed','failed')",
            [id.to_string()],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }

    pub fn set_options(&self, id: TaskId, options: &TaskOptions) -> Result<()> {
        let encrypted = secrets::protect(&serde_json::to_vec(options)?)?;
        let changed = self.connection()?.execute(
            "UPDATE tasks SET options = ?1 WHERE id = ?2 AND state IN ('queued','paused','failed')",
            params![encrypted, id.to_string()],
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

    pub(crate) fn fail_unexpected(&self, id: TaskId) -> Result<()> {
        self.connection()?.execute(
            "UPDATE tasks SET state = 'failed', error = 'download worker failed unexpectedly' WHERE id = ?1 AND state IN ('queued', 'running')",
            [id.to_string()],
        )?;
        Ok(())
    }
}

fn read_task(conn: &Connection, id: TaskId) -> Result<Option<Task>> {
    let row = conn
        .query_row(
            "SELECT request, state, output, error, size_verified, options FROM tasks WHERE id = ?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<bool>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(encrypted, state, output, error, size_verified, options)| {
            decode_task(
                id,
                &encrypted,
                &state,
                output,
                error,
                size_verified,
                options,
            )
        },
    )
    .transpose()
}

fn decode_task(
    id: TaskId,
    encrypted: &[u8],
    state: &str,
    output: Option<Vec<u8>>,
    error: Option<String>,
    size_verified: Option<bool>,
    options: Option<Vec<u8>>,
) -> Result<Task> {
    let request = serde_json::from_slice(&secrets::unprotect(encrypted)?)?;
    let output_path = output
        .map(|value| {
            let plaintext = secrets::unprotect(&value)?;
            Ok::<_, Error>(serde_json::from_slice(&plaintext)?)
        })
        .transpose()?;
    let options = options
        .map(|encrypted| Ok::<_, Error>(serde_json::from_slice(&secrets::unprotect(&encrypted)?)?))
        .transpose()?
        .unwrap_or_default();
    Ok(Task {
        id,
        request,
        options,
        state: TaskState::from_database(state)?,
        output_path,
        error,
        size_verified,
    })
}
