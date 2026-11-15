use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{
    DashOptions, Error, HlsOptions, NewTask, ProgressSnapshot, Result, SftpOptions, Task, TaskId,
    TaskOptions, TaskState, secrets,
};

pub struct Store {
    path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSftpHost {
    pub id: TaskId,
    pub observed: String,
    pub expected: Option<String>,
}

fn sftp_endpoint(url: &str) -> Result<(String, u16)> {
    let url = reqwest::Url::parse(url).map_err(|_| Error::InvalidSftpAddress)?;
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or(Error::InvalidSftpAddress)?;
    let port = url.port().unwrap_or(22);
    if url.scheme() != "sftp" || port == 0 {
        return Err(Error::InvalidSftpAddress);
    }
    Ok((host.to_ascii_lowercase(), port))
}

impl Store {
    /// Bind once; existing on-disk legacy data wins over a changed preference.
    pub fn assign_workspace(&self, id: TaskId, preferred: Option<&Path>) -> Result<Task> {
        let mut connection = self.connection()?;
        let task = read_task(&connection, id)?.ok_or(Error::NotFound)?;
        if task.cache_directory.is_some() {
            return Ok(task);
        }
        let legacy = task.request.destination.join(format!(".ndm-next-{id}"));
        let root = if legacy.try_exists()? {
            &task.request.destination
        } else {
            preferred.unwrap_or(&task.request.destination)
        };
        if !root.is_absolute() {
            return Err(std::io::Error::other("cache directory must be absolute").into());
        }
        // Network or redirected directories can be slow. Do not hold SQLite's
        // writer reservation during filesystem operations.
        std::fs::create_dir_all(root)?;
        let root = std::fs::canonicalize(root)?;
        let encrypted = secrets::protect(&serde_json::to_vec(&root)?)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read_task(&transaction, id)?.is_none() {
            return Err(Error::NotFound);
        }
        transaction.execute(
            "INSERT OR IGNORE INTO cache_workspaces (id, root) VALUES (?1, ?2)",
            params![id.to_string(), encrypted],
        )?;
        let task = read_task(&transaction, id)?.ok_or(Error::NotFound)?;
        transaction.commit()?;
        Ok(task)
    }

    pub(crate) fn cache_workspaces(&self) -> Result<Vec<(TaskId, PathBuf)>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare("SELECT id, root FROM cache_workspaces")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        rows.map(|row| {
            let (id, encrypted) = row?;
            let id = TaskId::parse_str(&id).map_err(|_| Error::InvalidStoredState)?;
            Ok((
                id,
                serde_json::from_slice(&secrets::unprotect(&encrypted)?)?,
            ))
        })
        .collect()
    }

    pub(crate) fn forget_cache_workspace(&self, id: TaskId) -> Result<()> {
        self.connection()?.execute("DELETE FROM cache_workspaces WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM tasks WHERE id = ?1)", [id.to_string()])?;
        Ok(())
    }

    pub fn reserve_update_if_idle(&self) -> Result<Option<crate::instance::DesktopUpdateGuard>> {
        let _admission = crate::instance::WorkAdmission::acquire(self.profile_directory())?;
        if crate::desktop_installation_active(self.profile_directory())? {
            return Ok(None);
        }
        let active: bool = self.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE state IN ('queued','running'))",
            [],
            |row| row.get(0),
        )?;
        if active {
            return Ok(None);
        }
        Ok(crate::instance::publish_update_reservation(
            self.profile_directory(),
        )?)
    }

    fn profile_directory(&self) -> &Path {
        self.path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    fn admit_work(&self) -> Result<crate::instance::WorkAdmission> {
        let admission = crate::instance::WorkAdmission::acquire(self.profile_directory())?;
        if crate::desktop_installation_active(self.profile_directory())? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "desktop update is in progress",
            )
            .into());
        }
        Ok(admission)
    }

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
        if version > 3 {
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
                progress BLOB,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE TABLE IF NOT EXISTS cache_workspaces (id TEXT PRIMARY KEY NOT NULL, root BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS discard_requests (id TEXT PRIMARY KEY NOT NULL);
            CREATE TABLE IF NOT EXISTS sftp_known_hosts (
                host TEXT NOT NULL, port INTEGER NOT NULL, fingerprint TEXT NOT NULL,
                PRIMARY KEY (host, port)
            );
            CREATE TABLE IF NOT EXISTS sftp_pending_hosts (
                id TEXT PRIMARY KEY NOT NULL, observed TEXT NOT NULL, expected TEXT
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
            ("progress", "BLOB"),
        ] {
            if !columns.iter().any(|column| column == name) {
                tx.execute(
                    &format!("ALTER TABLE tasks ADD COLUMN {name} {declaration}"),
                    [],
                )?;
            }
        }
        tx.pragma_update(None, "user_version", 3)?;
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
        let _admission = self.admit_work()?;
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

    /// Ask the desktop scheduler to stop a redundant browser handoff. The
    /// scheduler owns the worker and temporary files; the host must not delete
    /// them while a worker may still be running.
    pub fn request_discard(&self, id: TaskId) -> Result<Option<TaskState>> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = read_task(&tx, id)?.map(|task| task.state);
        if state.is_some_and(|state| state != TaskState::Completed) {
            tx.execute(
                "INSERT OR IGNORE INTO discard_requests (id) VALUES (?1)",
                [id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(state)
    }

    pub(crate) fn pending_discards(&self) -> Result<Vec<TaskId>> {
        let conn = self.connection()?;
        let mut statement = conn.prepare("SELECT id FROM discard_requests ORDER BY rowid")?;
        let ids = statement.query_map([], |row| row.get::<_, String>(0))?;
        ids.map(|id| TaskId::parse_str(&id?).map_err(|_| Error::InvalidStoredState))
            .collect()
    }

    pub(crate) fn finish_discard(&self, id: TaskId) -> Result<()> {
        self.connection()?.execute(
            "DELETE FROM discard_requests WHERE id = ?1",
            [id.to_string()],
        )?;
        Ok(())
    }

    pub fn get(&self, id: TaskId) -> Result<Option<Task>> {
        read_task(&self.connection()?, id)
    }

    pub fn list(&self) -> Result<Vec<Task>> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT id, request, state, output, error, size_verified, options, progress, (SELECT root FROM cache_workspaces w WHERE w.id = tasks.id) FROM tasks ORDER BY rowid",
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
            let progress: Option<Vec<u8>> = row.get(7)?;
            let cache_directory: Option<Vec<u8>> = row.get(8)?;
            tasks.push(decode_task(
                id,
                &encrypted,
                &state,
                output,
                error,
                size_verified,
                options,
                progress,
                cache_directory,
            )?);
        }
        Ok(tasks)
    }

    pub fn start(&self, id: TaskId) -> Result<()> {
        let _admission = self.admit_work()?;
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
        let _admission = self.admit_work()?;
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
        self.requeue(id, None)
    }

    pub(crate) fn resume_with_options(&self, id: TaskId, options: &TaskOptions) -> Result<()> {
        self.requeue(id, Some(options))
    }

    fn requeue(&self, id: TaskId, options: Option<&TaskOptions>) -> Result<()> {
        let _admission = self.admit_work()?;
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = if let Some(options) = options {
            let encrypted = secrets::protect(&serde_json::to_vec(options)?)?;
            tx.execute(
                "UPDATE tasks SET state = 'queued', error = NULL, options = ?1 WHERE id = ?2 AND state IN ('paused','failed')",
                params![encrypted, id.to_string()],
            )?
        } else {
            tx.execute(
            "UPDATE tasks SET state = 'queued', error = NULL WHERE id = ?1 AND state IN ('paused','failed')",
            [id.to_string()],
        )?
        };
        if changed == 1 {
            tx.execute(
                "DELETE FROM sftp_pending_hosts WHERE id = ?1",
                [id.to_string()],
            )?;
            tx.commit()?;
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
            TaskOptions::Auto if previous.path().to_ascii_lowercase().ends_with(".mpd") => {
                TaskOptions::Dash(DashOptions::default())
            }
            TaskOptions::Auto if previous.path().to_ascii_lowercase().ends_with(".m3u8") => {
                TaskOptions::Hls(HlsOptions::default())
            }
            TaskOptions::Auto => TaskOptions::AutoHttp,
            other => other,
        };
        let mut request = task.request;
        if matches!(options, TaskOptions::Dash(_)) {
            request.expected_size = None;
        }
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
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "DELETE FROM tasks WHERE id = ?1 AND state IN ('queued','paused','completed','failed')",
            [id.to_string()],
        )?;
        if changed == 1 {
            tx.execute(
                "DELETE FROM sftp_pending_hosts WHERE id = ?1",
                [id.to_string()],
            )?;
            tx.commit()?;
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

    pub fn sftp_known_fingerprint(&self, url: &str) -> Result<Option<String>> {
        let (host, port) = sftp_endpoint(url)?;
        Ok(self
            .connection()?
            .query_row(
                "SELECT fingerprint FROM sftp_known_hosts WHERE host = ?1 AND port = ?2",
                params![host, port],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn pending_sftp_hosts(&self) -> Result<Vec<PendingSftpHost>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare("SELECT p.id, p.observed, p.expected FROM sftp_pending_hosts p JOIN tasks t ON t.id = p.id WHERE t.state = 'failed' ORDER BY t.rowid")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.map(|row| {
            let (id, observed, expected) = row?;
            Ok(PendingSftpHost {
                id: TaskId::parse_str(&id).map_err(|_| Error::InvalidStoredState)?,
                observed,
                expected,
            })
        })
        .collect()
    }

    pub(crate) fn fail_sftp_host(
        &self,
        id: TaskId,
        observed: &str,
        expected: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reason = if expected.is_some() {
            "SFTP host key changed"
        } else {
            "SFTP host key needs confirmation"
        };
        if tx.execute(
            "UPDATE tasks SET state = 'failed', error = ?1 WHERE id = ?2 AND state = 'running'",
            params![reason, id.to_string()],
        )? != 1
        {
            return Err(Error::InvalidState);
        }
        tx.execute("INSERT OR REPLACE INTO sftp_pending_hosts (id, observed, expected) VALUES (?1, ?2, ?3)", params![id.to_string(), observed, expected])?;
        tx.commit()?;
        Ok(())
    }

    /// Only a fingerprint observed for this failed task may be confirmed.
    pub fn trust_sftp_host(&self, id: TaskId, observed: &str) -> Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = read_task(&tx, id)?.ok_or(Error::NotFound)?;
        if task.state != TaskState::Failed {
            return Err(Error::InvalidState);
        }
        let pending: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT observed, expected FROM sftp_pending_hosts WHERE id = ?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (pending_observed, expected) = pending.ok_or(Error::InvalidState)?;
        if pending_observed != observed {
            return Err(Error::InvalidState);
        }
        let (host, port) = sftp_endpoint(&task.request.url)?;
        let mut options = match task.options {
            TaskOptions::Auto => SftpOptions::default(),
            TaskOptions::Sftp(options) => options,
            _ => return Err(Error::InvalidState),
        };
        let known: Option<String> = tx
            .query_row(
                "SELECT fingerprint FROM sftp_known_hosts WHERE host = ?1 AND port = ?2",
                params![host, port],
                |row| row.get(0),
            )
            .optional()?;
        let current = known.or_else(|| options.trusted_fingerprint.clone());
        if current != expected && current.as_deref() != Some(observed) {
            tx.execute(
                "UPDATE sftp_pending_hosts SET expected = ?1 WHERE id = ?2",
                params![current, id.to_string()],
            )?;
            tx.commit()?;
            return Err(Error::InvalidState);
        }
        options.trusted_fingerprint = Some(observed.to_owned());
        let encrypted = secrets::protect(&serde_json::to_vec(&TaskOptions::Sftp(options))?)?;
        tx.execute(
            "INSERT OR REPLACE INTO sftp_known_hosts (host, port, fingerprint) VALUES (?1, ?2, ?3)",
            params![host, port, observed],
        )?;
        tx.execute(
            "UPDATE tasks SET options = ?1 WHERE id = ?2",
            params![encrypted, id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM sftp_pending_hosts WHERE id = ?1",
            [id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
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

    pub(crate) fn save_progress(&self, id: TaskId, progress: &ProgressSnapshot) -> Result<()> {
        let encrypted = secrets::protect(&serde_json::to_vec(progress)?)?;
        self.connection()?.execute(
            "UPDATE tasks SET progress = ?1 WHERE id = ?2",
            params![encrypted, id.to_string()],
        )?;
        Ok(())
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
            "SELECT request, state, output, error, size_verified, options, progress, (SELECT root FROM cache_workspaces w WHERE w.id = tasks.id) FROM tasks WHERE id = ?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<bool>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<Vec<u8>>>(7)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(encrypted, state, output, error, size_verified, options, progress, cache_directory)| {
            decode_task(
                id,
                &encrypted,
                &state,
                output,
                error,
                size_verified,
                options,
                progress,
                cache_directory,
            )
        },
    )
    .transpose()
}

#[allow(clippy::too_many_arguments)]
fn decode_task(
    id: TaskId,
    encrypted: &[u8],
    state: &str,
    output: Option<Vec<u8>>,
    error: Option<String>,
    size_verified: Option<bool>,
    options: Option<Vec<u8>>,
    progress: Option<Vec<u8>>,
    cache_directory: Option<Vec<u8>>,
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
    let progress = progress
        .map(|encrypted| Ok::<_, Error>(serde_json::from_slice(&secrets::unprotect(&encrypted)?)?))
        .transpose()?;
    let cache_directory = cache_directory
        .map(|bytes| Ok::<_, Error>(serde_json::from_slice(&secrets::unprotect(&bytes)?)?))
        .transpose()?;
    Ok(Task {
        cache_directory,
        id,
        request,
        options,
        state: TaskState::from_database(state)?,
        output_path,
        error,
        size_verified,
        progress,
    })
}
