use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_util::FutureExt;
use tokio::{
    sync::{Notify, broadcast, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{
    DashOptions, DownloadProgress, Error, FtpOptions, HlsOptions, HttpOptions, ProgressSnapshot,
    Result, SftpOptions, Store, Task, TaskId, TaskOptions, TaskState, execute_dash, execute_ftp,
    execute_hls, execute_http, execute_sftp,
};

#[derive(Clone, Debug)]
pub struct SchedulerOptions {
    pub max_active_tasks: usize,
    pub http: HttpOptions,
    pub auto_resume: bool,
}
impl Default for SchedulerOptions {
    fn default() -> Self {
        Self {
            max_active_tasks: 3,
            http: HttpOptions::default(),
            auto_resume: true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum SchedulerEvent {
    Started(TaskId),
    Finished {
        id: TaskId,
        progress: ProgressSnapshot,
    },
    SftpHostKeyRequired {
        id: TaskId,
        fingerprint: String,
    },
    SftpHostKeyChanged {
        id: TaskId,
        expected: String,
        observed: String,
    },
}

#[derive(Clone)]
struct Active {
    cancel: CancellationToken,
    stop: CancellationToken,
    manual_pause: Arc<AtomicBool>,
    progress: DownloadProgress,
    done: watch::Receiver<bool>,
}
struct Inner {
    store: Arc<Store>,
    options: RwLock<SchedulerOptions>,
    active: Mutex<HashMap<TaskId, Active>>,
    wake: Notify,
    shutdown: CancellationToken,
    events: broadcast::Sender<SchedulerEvent>,
    dispatcher: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct Scheduler(Arc<Inner>);

impl Scheduler {
    pub fn start(store: Arc<Store>, mut options: SchedulerOptions) -> Result<Self> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| Error::InvalidState)?;
        options.max_active_tasks = options.max_active_tasks.clamp(1, 32);
        if options.auto_resume {
            store.recover_interrupted()?;
        } else {
            store.recover_without_autostart()?;
        }
        let (events, _) = broadcast::channel(128);
        let inner = Arc::new(Inner {
            store,
            options: RwLock::new(options),
            active: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            shutdown: CancellationToken::new(),
            events,
            dispatcher: Mutex::new(None),
        });
        let dispatcher = runtime.spawn(dispatch(inner.clone()));
        *inner.dispatcher.lock().unwrap() = Some(dispatcher);
        Ok(Self(inner))
    }

    /// Wake the dispatcher after another component commits a task.
    pub fn notify(&self) {
        self.0.wake.notify_one();
    }

    /// Update defaults for queued and future tasks. Running workers retain their settings.
    pub fn update_options(&self, mut options: SchedulerOptions) {
        options.max_active_tasks = options.max_active_tasks.clamp(1, 32);
        *self.0.options.write().unwrap() = options;
        self.notify();
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SchedulerEvent> {
        self.0.events.subscribe()
    }

    pub fn progress(&self, id: TaskId) -> Option<ProgressSnapshot> {
        self.0
            .active
            .lock()
            .unwrap()
            .get(&id)
            .map(|active| active.progress.snapshot())
    }

    /// Remove a task only after its worker has stopped. Completed files are kept.
    pub fn delete(&self, id: TaskId, remove_partial: bool) -> Result<()> {
        // The dispatcher inserts workers under the same lock. Holding it until
        // the record is removed prevents a queued task from starting mid-delete.
        let active = self.0.active.lock().unwrap();
        if active.contains_key(&id) {
            return Err(Error::InvalidState);
        }
        let task = self.0.store.get(id)?.ok_or(Error::NotFound)?;
        if !matches!(
            task.state,
            TaskState::Queued | TaskState::Paused | TaskState::Failed | TaskState::Completed
        ) {
            return Err(Error::InvalidState);
        }
        if remove_partial && task.state != TaskState::Completed {
            remove_owned_partial_files(&task.request.destination, id)?;
        }
        self.0.store.delete(id)
    }

    pub async fn pause(&self, id: TaskId) -> Result<()> {
        let active = self.0.active.lock().unwrap().get(&id).cloned();
        if let Some(mut active) = active {
            active.manual_pause.store(true, Ordering::Release);
            active.cancel.cancel();
            while !*active.done.borrow_and_update() {
                if active.done.changed().await.is_err() {
                    break;
                }
            }
            match self.0.store.get(id)? {
                Some(task) if task.state == TaskState::Paused => Ok(()),
                _ => Err(Error::InvalidState),
            }
        } else {
            self.0.store.pause(id)
        }
    }

    pub fn stop_live(&self, id: TaskId) -> Result<()> {
        let active = self
            .0
            .active
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or(Error::InvalidState)?;
        active.stop.cancel();
        Ok(())
    }

    pub async fn resume(&self, id: TaskId) -> Result<()> {
        self.0.store.resume(id)?;
        self.notify();
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.0.shutdown.cancel();
        self.0.wake.notify_one();
        let dispatcher = self.0.dispatcher.lock().unwrap().take();
        if let Some(dispatcher) = dispatcher {
            let _ = dispatcher.await;
        }
        let active: Vec<_> = self.0.active.lock().unwrap().values().cloned().collect();
        for job in &active {
            job.cancel.cancel();
        }
        for mut job in active {
            while !*job.done.borrow_and_update() {
                if job.done.changed().await.is_err() {
                    break;
                }
            }
        }
    }

    async fn discard_copy(&self, id: TaskId) -> Result<()> {
        if self.0.active.lock().unwrap().contains_key(&id) {
            // If completion races with cancellation, keep the completed file.
            let _ = self.pause(id).await;
        }
        match self.0.store.get(id)? {
            None
            | Some(Task {
                state: TaskState::Completed,
                ..
            }) => {}
            Some(Task {
                state: TaskState::Prepared,
                ..
            }) => self.0.store.abort(id)?,
            Some(Task {
                state: TaskState::Queued | TaskState::Paused | TaskState::Failed,
                ..
            }) => self.delete(id, true)?,
            Some(_) => return Err(Error::InvalidState),
        }
        Ok(())
    }
}

fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn remove_owned_partial_files(destination: &Path, id: TaskId) -> Result<()> {
    let directory = destination.join(format!(".ndm-next-{id}"));
    let metadata = match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() || is_reparse_point(&metadata) {
        return Err(io::Error::other("temporary directory is not a regular directory").into());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let known = matches!(
            name.as_ref(),
            "resume.bin"
                | "assembled.part"
                | "ftp.part"
                | "sftp.part"
                | "protocol-resume.bin"
                | "protocol-resume.pending"
                | "hls-output.mp4"
                | "hls-output.mp4.hls-partial"
                | "hls-checkpoint.sqlite"
                | "hls-checkpoint.sqlite-journal"
        ) || name
            .strip_prefix("segment-")
            .and_then(|name| name.strip_suffix(".part"))
            .and_then(|index| index.parse::<usize>().ok())
            .is_some_and(|index| index < 16 && name == format!("segment-{index}.part"))
            || crate::dash::owns_partial_name(&name);
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if !known || !metadata.is_file() || is_reparse_point(&metadata) {
            return Err(
                io::Error::other("temporary directory contains an unexpected entry").into(),
            );
        }
        files.push(entry.path());
    }
    for file in files {
        std::fs::remove_file(file)?;
    }
    std::fs::remove_dir(directory)?;
    Ok(())
}

async fn dispatch(inner: Arc<Inner>) {
    loop {
        if inner.shutdown.is_cancelled() {
            return;
        }
        let notified = inner.wake.notified();
        if let Ok(requests) = inner.store.pending_discards()
            && let Some(id) = requests.first().copied()
        {
            // Failed cleanup keeps the task and its files for the user; it must
            // not spin forever or prevent unrelated downloads from starting.
            let _ = Scheduler(inner.clone()).discard_copy(id).await;
            let _ = inner.store.finish_discard(id);
            continue;
        }
        let capacity = inner
            .options
            .read()
            .unwrap()
            .max_active_tasks
            .saturating_sub(inner.active.lock().unwrap().len());
        if capacity > 0 {
            let queued = match inner.store.list() {
                Ok(tasks) => tasks,
                Err(_) => {
                    tokio::select! { _ = inner.shutdown.cancelled() => return, _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {} }
                    continue;
                }
            };
            let mut started = 0;
            for task in queued
                .into_iter()
                .filter(|task| task.state == TaskState::Queued)
            {
                if started >= capacity || inner.shutdown.is_cancelled() {
                    break;
                }
                let mut active = inner.active.lock().unwrap();
                if active.contains_key(&task.id) {
                    continue;
                }
                let (done_tx, done) = watch::channel(false);
                let job = Active {
                    cancel: CancellationToken::new(),
                    stop: CancellationToken::new(),
                    manual_pause: Arc::new(AtomicBool::new(false)),
                    progress: DownloadProgress::default(),
                    done,
                };
                active.insert(task.id, job.clone());
                drop(active);
                started += 1;
                tokio::spawn(run_one(inner.clone(), task, job, done_tx));
            }
            if started > 0 {
                continue;
            }
        }
        tokio::select! { _ = inner.shutdown.cancelled() => return, _ = notified => {} }
    }
}

async fn run_one(inner: Arc<Inner>, task: Task, active: Active, done: watch::Sender<bool>) {
    let _ = inner.events.send(SchedulerEvent::Started(task.id));
    let result = std::panic::AssertUnwindSafe(execute_one(&inner, &task, &active))
        .catch_unwind()
        .await;
    match result {
        Ok(Err(Error::SftpUnknownHost { fingerprint })) => {
            let _ = inner.events.send(SchedulerEvent::SftpHostKeyRequired {
                id: task.id,
                fingerprint,
            });
        }
        Ok(Err(Error::SftpHostKeyChanged { expected, observed })) => {
            let _ = inner.events.send(SchedulerEvent::SftpHostKeyChanged {
                id: task.id,
                expected,
                observed,
            });
        }
        Ok(Err(_)) | Err(_) => {
            let _ = inner.store.fail_unexpected(task.id);
        }
        Ok(Ok(())) => {}
    }
    if inner.shutdown.is_cancelled()
        && !active.manual_pause.load(Ordering::Acquire)
        && inner
            .store
            .get(task.id)
            .ok()
            .flatten()
            .is_some_and(|task| task.state == TaskState::Paused)
    {
        let _ = inner.store.resume(task.id);
    }
    let progress = active.progress.snapshot();
    inner.active.lock().unwrap().remove(&task.id);
    let _ = done.send(true);
    let _ = inner.events.send(SchedulerEvent::Finished {
        id: task.id,
        progress,
    });
    inner.wake.notify_one();
}

async fn execute_one(inner: &Arc<Inner>, task: &Task, active: &Active) -> Result<()> {
    let defaults = inner.options.read().unwrap().clone();
    let options = match task.options.clone() {
        TaskOptions::AutoHttp => TaskOptions::Http(defaults.http.clone()),
        TaskOptions::Auto => {
            match reqwest::Url::parse(&task.request.url).ok().map(|url| {
                let scheme = url.scheme().to_owned();
                let path = url.path().to_ascii_lowercase();
                (scheme, path)
            }) {
                Some((scheme, _)) if scheme == "ftp" || scheme == "ftps" => {
                    TaskOptions::Ftp(FtpOptions {
                        max_bytes_per_second: defaults.http.max_bytes_per_second,
                        ..Default::default()
                    })
                }
                Some((scheme, _)) if scheme == "sftp" => TaskOptions::Sftp(SftpOptions {
                    max_bytes_per_second: defaults.http.max_bytes_per_second,
                    ..Default::default()
                }),
                Some((scheme, path))
                    if (scheme == "http" || scheme == "https") && path.ends_with(".mpd") =>
                {
                    TaskOptions::Dash(DashOptions {
                        max_bytes_per_second: defaults.http.max_bytes_per_second,
                        proxy: defaults.http.proxy.clone(),
                        ..Default::default()
                    })
                }
                Some((scheme, path))
                    if (scheme == "http" || scheme == "https") && path.ends_with(".m3u8") =>
                {
                    TaskOptions::Hls(HlsOptions {
                        max_bytes_per_second: defaults.http.max_bytes_per_second,
                        proxy: defaults.http.proxy.clone(),
                        ..Default::default()
                    })
                }
                Some((scheme, _)) if scheme == "http" || scheme == "https" => {
                    TaskOptions::Http(defaults.http.clone())
                }
                _ => TaskOptions::Auto,
            }
        }
        TaskOptions::Dash(mut options) => {
            options.proxy = options.proxy.or(defaults.http.proxy.clone());
            options.max_bytes_per_second = options
                .max_bytes_per_second
                .or(defaults.http.max_bytes_per_second);
            TaskOptions::Dash(options)
        }
        TaskOptions::Hls(mut options) => {
            options.proxy = options.proxy.or(defaults.http.proxy.clone());
            options.max_bytes_per_second = options
                .max_bytes_per_second
                .or(defaults.http.max_bytes_per_second);
            TaskOptions::Hls(options)
        }
        TaskOptions::Sftp(mut options) => {
            options.max_bytes_per_second = options
                .max_bytes_per_second
                .or(defaults.http.max_bytes_per_second);
            TaskOptions::Sftp(options)
        }
        TaskOptions::Ftp(mut options) => {
            options.max_bytes_per_second = options
                .max_bytes_per_second
                .or(defaults.http.max_bytes_per_second);
            TaskOptions::Ftp(options)
        }
        other => other,
    };
    match options {
        TaskOptions::Auto | TaskOptions::AutoHttp => match inner.store.start(task.id) {
            Ok(()) => {
                let _ = inner.store.fail(task.id, "unsupported URL protocol");
                Err(Error::InvalidState)
            }
            Err(error) => Err(error),
        },
        TaskOptions::Http(options) => execute_http(
            &inner.store,
            task.id,
            &options,
            active.cancel.clone(),
            active.progress.clone(),
        )
        .await
        .map(|_| ()),
        TaskOptions::Ftp(options) => execute_ftp(
            &inner.store,
            task.id,
            &options,
            active.cancel.clone(),
            active.progress.clone(),
        )
        .await
        .map(|_| ()),
        TaskOptions::Sftp(mut options) => {
            if let Some(fingerprint) = inner.store.sftp_known_fingerprint(&task.request.url)? {
                options.trusted_fingerprint = Some(fingerprint);
            }
            execute_sftp(
                &inner.store,
                task.id,
                &options,
                active.cancel.clone(),
                active.progress.clone(),
            )
            .await
            .map(|_| ())
        }
        TaskOptions::Dash(options) => execute_dash(
            &inner.store,
            task.id,
            &options,
            active.cancel.clone(),
            active.progress.clone(),
        )
        .await
        .map(|_| ()),
        TaskOptions::Hls(options) => {
            // hls-engine uses non-Send futures, so its bounded worker owns a
            // current-thread runtime instead of moving those futures between threads.
            let store = inner.store.clone();
            let id = task.id;
            let cancel = active.cancel.clone();
            let stop = active.stop.clone();
            let progress = active.progress.clone();
            match tokio::task::spawn_blocking(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime
                    .block_on(execute_hls(&store, id, &options, cancel, stop, progress))
                    .map(|_| ())
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(Error::Hls),
            }
        }
    }
}
