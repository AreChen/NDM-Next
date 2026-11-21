use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use futures_util::FutureExt;
use tokio::{
    sync::{Notify, broadcast, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{
    DashOptions, DownloadLimits, DownloadProgress, Error, FtpOptions, HlsOptions, HttpOptions,
    ProgressSnapshot, ProxyOptions, Result, SftpOptions, Store, Task, TaskCommand, TaskId,
    TaskOptions, TaskState, execute_dash, execute_ftp, execute_hls, execute_http, execute_sftp,
};

#[derive(Clone, Debug)]
pub struct SchedulerOptions {
    pub max_active_tasks: usize,
    pub http: HttpOptions,
    pub auto_resume: bool,
    pub cache_directory: Option<PathBuf>,
}
impl SchedulerOptions {
    pub fn apply_limits(&mut self, limits: DownloadLimits) {
        self.max_active_tasks = limits.active_tasks as usize;
        self.http.max_connections = limits.download_threads as usize;
    }
}
impl Default for SchedulerOptions {
    fn default() -> Self {
        Self {
            max_active_tasks: 3,
            http: HttpOptions::default(),
            auto_resume: true,
            cache_directory: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum SchedulerEvent {
    Started(TaskId),
    TaskTitleChanged(TaskId),
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
    batch: Option<TaskId>,
    plugin: Option<String>,
    cancel: CancellationToken,
    stop: CancellationToken,
    manual_pause: Arc<AtomicBool>,
    progress: DownloadProgress,
    done: watch::Receiver<bool>,
    started: Instant,
    previous_elapsed_ms: u64,
    previous_transferred: u64,
}

impl Active {
    fn snapshot(&self) -> ProgressSnapshot {
        let mut progress = self.progress.snapshot();
        progress.elapsed_ms = self
            .previous_elapsed_ms
            .saturating_add(self.started.elapsed().as_millis().min(u64::MAX as u128) as u64);
        progress.transferred_total = self
            .previous_transferred
            .saturating_add(progress.transferred);
        progress
    }
}
struct Inner {
    store: Arc<Store>,
    options: RwLock<SchedulerOptions>,
    active: Mutex<HashMap<TaskId, Active>>,
    maintenance: RwLock<()>,
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
        if let Some(limits) = store.download_limits()? {
            options.apply_limits(limits);
        }
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
            maintenance: RwLock::new(()),
            wake: Notify::new(),
            shutdown: CancellationToken::new(),
            events,
            dispatcher: Mutex::new(None),
        });
        let dispatcher = runtime.spawn(dispatch(inner.clone()));
        *inner.dispatcher.lock().unwrap() = Some(dispatcher);
        Ok(Self(inner))
    }

    pub fn cache_usage(&self) -> Result<crate::CacheReport> {
        self.maintain_cache(None, false)
    }

    /// Only completed/unreferenced workspaces are disposable. Never purge resumable tasks.
    /// `cutoff` is the latest allowed modification time; `None` means manual cleanup.
    pub fn clean_cache(&self, cutoff: Option<std::time::SystemTime>) -> Result<crate::CacheReport> {
        self.maintain_cache(cutoff, true)
    }

    fn maintain_cache(
        &self,
        cutoff: Option<std::time::SystemTime>,
        remove: bool,
    ) -> Result<crate::CacheReport> {
        let mut workspaces: HashMap<_, _> = self.0.store.cache_workspaces()?.into_iter().collect();
        for task in self.0.store.list()? {
            workspaces
                .entry(task.id)
                .or_insert_with(|| task.cache_directory.unwrap_or(task.request.destination));
        }
        let mut report = crate::CacheReport::default();
        for (id, root) in workspaces {
            // Starting workers also hold the maintenance gate while binding their workspace.
            // Progress reads do not use this gate and remain responsive during cleanup.
            let _maintenance = self.0.maintenance.write().unwrap();
            let task = self.0.store.get(id)?;
            let protected = task
                .as_ref()
                .is_some_and(|task| task.state != TaskState::Completed)
                || self.0.active.lock().unwrap().contains_key(&id);
            let files = match crate::cache::inspect(&root, id) {
                Ok(Some(files)) => files,
                Ok(None) => {
                    if remove && task.is_none() {
                        self.0.store.forget_cache_workspace(id)?;
                    }
                    continue;
                }
                Err(_) => {
                    report.skipped_directories += 1;
                    continue;
                }
            };
            report.bytes = report.bytes.saturating_add(files.bytes);
            if protected {
                report.protected_bytes = report.protected_bytes.saturating_add(files.bytes);
                continue;
            }
            report.reclaimable_bytes = report.reclaimable_bytes.saturating_add(files.bytes);
            if remove && cutoff.is_none_or(|cutoff| files.modified <= cutoff) {
                let bytes = files.bytes;
                match files.remove() {
                    Ok(count) => {
                        report.removed_bytes = report.removed_bytes.saturating_add(bytes);
                        report.removed_files += count;
                        if task.is_none() {
                            self.0.store.forget_cache_workspace(id)?;
                        }
                    }
                    Err(_) => report.skipped_directories += 1,
                }
            }
        }
        Ok(report)
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

    pub fn refresh_download_limits(&self) {
        if let Ok(Some(limits)) = self.0.store.download_limits() {
            self.0.options.write().unwrap().apply_limits(limits);
        }
        self.notify();
    }

    pub fn uninstall_plugin(&self, id: &str) -> Result<()> {
        let _maintenance = self.0.maintenance.write().unwrap();
        if self
            .0
            .active
            .lock()
            .unwrap()
            .values()
            .any(|worker| worker.plugin.as_deref() == Some(id))
            || self
                .0
                .store
                .list_queued()?
                .iter()
                .any(|task| plugin_id(task) == Some(id))
        {
            return Err(Error::InvalidState);
        }
        crate::plugins::PluginRegistry::new(self.0.store.profile_directory()).remove(id)?;
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SchedulerEvent> {
        self.0.events.subscribe()
    }

    pub fn progress(&self, id: TaskId) -> Option<ProgressSnapshot> {
        self.0.active.lock().unwrap().get(&id).map(Active::snapshot)
    }

    /// Remove a task only after its worker has stopped. Completed files are kept.
    pub fn delete(&self, id: TaskId, remove_partial: bool) -> Result<()> {
        // Block dispatch while deleting files without holding the progress-map lock.
        let _maintenance = self.0.maintenance.write().unwrap();
        let active = self.0.active.lock().unwrap();
        if active.contains_key(&id) {
            return Err(Error::InvalidState);
        }
        drop(active);
        let task = self.0.store.get(id)?.ok_or(Error::NotFound)?;
        if task.cache_directory.is_none() && task.work_directory().try_exists()? {
            self.0.store.assign_workspace(id, None)?;
        }
        if !matches!(
            task.state,
            TaskState::Queued | TaskState::Paused | TaskState::Failed | TaskState::Completed
        ) {
            return Err(Error::InvalidState);
        }
        if remove_partial {
            crate::cache::remove_partial(
                task.cache_directory
                    .as_deref()
                    .unwrap_or(&task.request.destination),
                id,
            )?;
        }
        self.0.store.delete(id)
    }

    pub async fn pause(&self, id: TaskId) -> Result<()> {
        let active = {
            let workers = self.0.active.lock().unwrap();
            let active = workers.get(&id).cloned();
            if active.is_none() {
                self.0.store.pause(id)?;
            }
            active
        };
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
            Ok(())
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

    /// Restart only this task through the supplied proxy, retaining its request,
    /// protocol selections and resumable data. Credentials remain encrypted by Store.
    pub async fn retry_with_proxy(&self, id: TaskId, proxy: ProxyOptions) -> Result<()> {
        proxy.validate()?;
        let task = self.0.store.get(id)?.ok_or(Error::NotFound)?;
        task_proxy_options(&task, &self.0.options.read().unwrap(), proxy.clone())?;
        if self.progress(id).is_some()
            || matches!(task.state, TaskState::Queued | TaskState::Running)
        {
            self.pause(id).await?;
        }
        // The dispatcher revalidates state and options under this same lock.
        // No worker can start between updating the encrypted options and queuing.
        {
            let active = self.0.active.lock().unwrap();
            if active.contains_key(&id) {
                return Err(Error::InvalidState);
            }
            let task = self.0.store.get(id)?.ok_or(Error::NotFound)?;
            let options = task_proxy_options(&task, &self.0.options.read().unwrap(), proxy)?;
            self.0.store.resume_with_options(id, &options)?;
        }
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

async fn dispatch(inner: Arc<Inner>) {
    loop {
        if inner.shutdown.is_cancelled() {
            return;
        }
        let notified = inner.wake.notified();
        if let Ok(Some((sequence, id, command))) = inner.store.next_task_command() {
            let scheduler = Scheduler(inner.clone());
            let _ = match command {
                TaskCommand::Pause => scheduler.pause(id).await,
                TaskCommand::Resume => scheduler.resume(id).await,
            };
            if inner.store.finish_task_command(sequence).is_ok() {
                continue;
            }
        }
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
            let queued = match inner.store.list_queued() {
                Ok(tasks) => tasks,
                Err(_) => {
                    tokio::select! { _ = inner.shutdown.cancelled() => return, _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {} }
                    continue;
                }
            };
            let mut started = 0;
            let mut engine_limits = HashMap::new();
            for task in queued {
                if started >= capacity || inner.shutdown.is_cancelled() {
                    break;
                }
                let _maintenance = inner.maintenance.read().unwrap();
                let Ok(Some(task)) = inner.store.get(task.id) else {
                    continue;
                };
                if task.state != TaskState::Queued {
                    continue;
                }
                let plugin = plugin_id(&task);
                let plugin_limit = plugin.and_then(|id| {
                    *engine_limits.entry(id.to_owned()).or_insert_with(|| {
                        crate::plugins::PluginRegistry::new(inner.store.profile_directory())
                            .get(id)
                            .ok()
                            .flatten()
                            .map(|engine| engine.manifest.limits.active_tasks as usize)
                    })
                });
                let mut active = inner.active.lock().unwrap();
                if active.contains_key(&task.id)
                    || task.policy.batch.is_some_and(|batch| {
                        active
                            .values()
                            .filter(|job| job.batch == Some(batch.id))
                            .count()
                            >= batch.active_tasks as usize
                    })
                    || plugin_limit.is_some_and(|limit| {
                        active
                            .values()
                            .filter(|job| job.plugin.as_deref() == plugin)
                            .count()
                            >= limit
                    })
                {
                    continue;
                }
                let (done_tx, done) = watch::channel(false);
                let job = Active {
                    batch: task.policy.batch.map(|batch| batch.id),
                    plugin: plugin.map(str::to_owned),
                    cancel: CancellationToken::new(),
                    stop: CancellationToken::new(),
                    manual_pause: Arc::new(AtomicBool::new(false)),
                    progress: DownloadProgress::default(),
                    done,
                    started: Instant::now(),
                    previous_elapsed_ms: task
                        .progress
                        .as_ref()
                        .map_or(0, |progress| progress.elapsed_ms),
                    previous_transferred: task
                        .progress
                        .as_ref()
                        .map_or(0, |progress| progress.transferred_total),
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

fn plugin_id(task: &Task) -> Option<&str> {
    match &task.options {
        TaskOptions::Plugin(options) => Some(&options.id),
        _ => None,
    }
}

async fn run_one(inner: Arc<Inner>, task: Task, active: Active, done: watch::Sender<bool>) {
    let _ = inner.events.send(SchedulerEvent::Started(task.id));
    let result = std::panic::AssertUnwindSafe(async {
        {
            let _maintenance = inner.maintenance.read().unwrap();
            let preferred = inner.options.read().unwrap().cache_directory.clone();
            inner
                .store
                .assign_workspace(task.id, preferred.as_deref())?;
        }
        execute_one(&inner, &task, &active).await
    })
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
    let mut progress = active.snapshot();
    for segment in &mut progress.segments {
        segment.active = false;
    }
    if let Ok(Some(task)) = inner.store.get(task.id) {
        if task.state == TaskState::Completed {
            if let Some(output) = task.output_path
                && let Ok(metadata) = std::fs::metadata(output)
                && metadata.is_file()
            {
                progress.downloaded = metadata.len();
                progress.total = Some(metadata.len());
            }
        } else if progress.downloaded == 0
            && let Some(saved) = crate::task_progress::recover_partial_progress(&task)
        {
            // A cancelled/offline probe has not restored the protocol counters yet.
            // Read the remaining owned parts rather than overwriting their progress with zero.
            progress.downloaded = saved.downloaded;
            progress.total = progress.total.or(saved.total);
            if progress.segments.is_empty() {
                progress.segments = saved.segments;
            }
        }
    }
    if let Err(error) = inner.store.save_progress(task.id, &progress) {
        eprintln!("Could not save download progress for {}: {error}", task.id);
    }
    inner.active.lock().unwrap().remove(&task.id);
    let _ = done.send(true);
    let _ = inner.events.send(SchedulerEvent::Finished {
        id: task.id,
        progress,
    });
    inner.wake.notify_one();
}

fn task_proxy_options(
    task: &Task,
    defaults: &SchedulerOptions,
    proxy: ProxyOptions,
) -> Result<TaskOptions> {
    let mut options = task_options(task, defaults);
    match &mut options {
        TaskOptions::Http(options) => options.proxy = Some(proxy),
        TaskOptions::Hls(options) => options.proxy = Some(proxy),
        TaskOptions::Dash(options) => options.proxy = Some(proxy),
        _ => return Err(Error::InvalidState),
    }
    Ok(options)
}

fn task_options(task: &Task, defaults: &SchedulerOptions) -> TaskOptions {
    match task.options.clone() {
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
    }
}

async fn execute_one(inner: &Arc<Inner>, task: &Task, active: &Active) -> Result<()> {
    let (options, default_threads) = {
        let defaults = inner.options.read().unwrap();
        (
            task_options(task, &defaults),
            defaults.http.max_connections.clamp(1, 256) as u32,
        )
    };
    match options {
        TaskOptions::Auto | TaskOptions::AutoHttp => match inner.store.start(task.id) {
            Ok(()) => {
                let _ = inner.store.fail(task.id, "unsupported URL protocol");
                Err(Error::InvalidState)
            }
            Err(error) => Err(error),
        },
        TaskOptions::Http(mut options) => {
            if let Some(threads) = task.policy.download_threads {
                options.max_connections = threads as usize;
            }
            execute_http(
                &inner.store,
                task.id,
                &options,
                active.cancel.clone(),
                active.progress.clone(),
            )
            .await
            .map(|_| ())
        }
        TaskOptions::Plugin(options) => {
            crate::plugin_transfer::execute(
                &inner.store,
                task.id,
                options,
                default_threads,
                active.cancel.clone(),
                active.progress.clone(),
                || {
                    let _ = inner.events.send(SchedulerEvent::TaskTitleChanged(task.id));
                },
            )
            .await
        }
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
        TaskOptions::Hls(mut options) => {
            options.download_threads = task
                .policy
                .download_threads
                .or(options.download_threads)
                .or(Some(default_threads));
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

#[cfg(test)]
mod proxy_tests {
    use super::*;

    #[test]
    fn proxy_restart_preserves_protocol_tracks_limits_and_explicit_http_detection() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("tasks.db")).unwrap();
        let id = TaskId::new_v4();
        store
            .prepare(
                id,
                &crate::NewTask {
                    url: "https://example.test/media.m3u8".into(),
                    destination: dir.path().into(),
                    suggested_filename: None,
                    source_url: None,
                    expected_size: None,
                    headers: Default::default(),
                },
            )
            .unwrap();
        let mut task = store.get(id).unwrap().unwrap();
        let proxy = ProxyOptions {
            url: "http://127.0.0.1:18085".into(),
            username: None,
            password: None,
        };
        let defaults = SchedulerOptions::default();
        task.options = TaskOptions::Hls(HlsOptions {
            variant_index: Some(3),
            audio_names: vec!["中文".into(), "English".into()],
            max_bytes_per_second: Some(512),
            ..Default::default()
        });
        let TaskOptions::Hls(options) =
            task_proxy_options(&task, &defaults, proxy.clone()).unwrap()
        else {
            panic!("lost HLS selections");
        };
        assert_eq!(options.variant_index, Some(3));
        assert_eq!(options.audio_names, ["中文", "English"]);
        assert_eq!(options.max_bytes_per_second, Some(512));
        assert_eq!(options.proxy.as_ref(), Some(&proxy));
        task.options = TaskOptions::Dash(DashOptions {
            representation_index: Some(2),
            audio_indices: vec![1, 4],
            ..Default::default()
        });
        let TaskOptions::Dash(options) =
            task_proxy_options(&task, &defaults, proxy.clone()).unwrap()
        else {
            panic!("lost DASH selections");
        };
        assert_eq!(options.representation_index, Some(2));
        assert_eq!(options.audio_indices, [1, 4]);
        assert_eq!(options.proxy.as_ref(), Some(&proxy));
        task.options = TaskOptions::Http(HttpOptions {
            max_connections: 7,
            max_bytes_per_second: Some(1024),
            proxy: None,
        });
        let TaskOptions::Http(options) =
            task_proxy_options(&task, &defaults, proxy.clone()).unwrap()
        else {
            panic!("lost HTTP options");
        };
        assert_eq!(options.max_connections, 7);
        assert_eq!(options.max_bytes_per_second, Some(1024));
        task.options = TaskOptions::AutoHttp;
        assert!(matches!(
            task_proxy_options(&task, &defaults, proxy.clone()).unwrap(),
            TaskOptions::Http(_)
        ));
        task.options = TaskOptions::Auto;
        assert!(matches!(
            task_proxy_options(&task, &defaults, proxy.clone()).unwrap(),
            TaskOptions::Hls(_)
        ));
        task.request.url = "sftp://example.test/file".into();
        assert!(task_proxy_options(&task, &defaults, proxy).is_err());
    }
}
