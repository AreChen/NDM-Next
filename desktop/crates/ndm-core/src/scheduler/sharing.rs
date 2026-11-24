use super::*;
use crate::{
    PeerStats, cache,
    plugin_transfer::build_request,
    plugins::{InstalledPlugin, PluginRegistry},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharingStatus {
    Checking,
    Sharing,
    Stopping,
}

#[derive(Clone)]
pub(super) struct ActiveShare {
    pub(super) plugin: String,
    cancel: CancellationToken,
    progress: Arc<Mutex<ProgressSnapshot>>,
    ready: Arc<AtomicBool>,
    done: watch::Receiver<bool>,
}

impl ActiveShare {
    pub(super) fn snapshot(&self) -> ProgressSnapshot {
        self.progress.lock().unwrap().clone()
    }

    pub(super) async fn stop(mut self) {
        self.cancel.cancel();
        while !*self.done.borrow_and_update() {
            if self.done.changed().await.is_err() {
                break;
            }
        }
    }

    fn status(&self) -> SharingStatus {
        if self.cancel.is_cancelled() {
            SharingStatus::Stopping
        } else if self.ready.load(Ordering::Acquire) {
            SharingStatus::Sharing
        } else {
            SharingStatus::Checking
        }
    }
}

pub(super) fn shareable_engines(installed: &[InstalledPlugin]) -> HashSet<String> {
    installed
        .iter()
        .filter(|plugin| plugin.manifest.capabilities.share_completed)
        .map(|plugin| plugin.manifest.id.clone())
        .collect()
}

impl Scheduler {
    pub async fn set_sharing(&self, id: TaskId, enabled: bool) -> Result<()> {
        if enabled {
            self.start_sharing(id)
        } else {
            self.stop_sharing(id).await;
            Ok(())
        }
    }

    pub fn update_plugin_capabilities(&self, installed: &[InstalledPlugin]) {
        *self.0.shareable.write().unwrap() = shareable_engines(installed);
    }

    pub fn can_share(&self, task: &Task) -> bool {
        task.state == TaskState::Completed
            && task.output_path.is_some()
            && plugin_id(task).is_some_and(|id| self.0.shareable.read().unwrap().contains(id))
    }

    pub fn sharing_status(&self, id: TaskId) -> Option<SharingStatus> {
        self.0
            .sharing
            .lock()
            .unwrap()
            .get(&id)
            .map(ActiveShare::status)
    }

    pub fn start_sharing(&self, id: TaskId) -> Result<()> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| Error::InvalidState)?;
        let _maintenance = self.0.maintenance.write().unwrap();
        if self.0.shutdown.is_cancelled() || self.0.active.lock().unwrap().contains_key(&id) {
            return Err(Error::InvalidState);
        }
        if self.0.sharing.lock().unwrap().contains_key(&id) {
            return Ok(());
        }
        let task = self.0.store.get(id)?.ok_or(Error::NotFound)?;
        let TaskOptions::Plugin(options) = &task.options else {
            return Err(Error::PluginOptions);
        };
        if task.state != TaskState::Completed {
            return Err(Error::InvalidState);
        }
        let output = task.output_path.clone().ok_or(Error::PluginStorage)?;
        let file = std::fs::symlink_metadata(&output)?;
        if cache::is_reparse(&file)
            || !(file.is_file() || file.is_dir())
            || output
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                != Some(task.request.destination.canonicalize()?)
        {
            return Err(Error::PluginStorage);
        }
        let plugin = PluginRegistry::new(self.0.store.profile_directory())
            .get(&options.id)?
            .ok_or(Error::PluginUnavailable)?;
        if !plugin.manifest.capabilities.share_completed {
            return Err(Error::PluginOptions);
        }
        if self
            .0
            .sharing
            .lock()
            .unwrap()
            .values()
            .filter(|job| job.plugin == options.id)
            .count()
            >= plugin.manifest.limits.active_tasks as usize
        {
            return Err(Error::InvalidState);
        }
        let engine = plugin.engine()?;
        let workspace = cache::engine_workspace(&task)?;
        let threads = self.0.options.read().unwrap().http.max_connections as u32;
        let request = build_request(&task, &plugin, options.clone(), threads, workspace)?;
        let mut progress = task.progress.unwrap_or_else(|| ProgressSnapshot {
            downloaded: if file.is_file() { file.len() } else { 0 },
            total: file.is_file().then_some(file.len()),
            ..Default::default()
        });
        let saved_upload = progress.peers.map_or(0, |stats| stats.uploaded);
        progress.peers = Some(PeerStats {
            uploaded: saved_upload,
            ..Default::default()
        });
        let (done_tx, done) = watch::channel(false);
        let job = ActiveShare {
            plugin: options.id.clone(),
            cancel: self.0.shutdown.child_token(),
            progress: Arc::new(Mutex::new(progress)),
            ready: Arc::default(),
            done,
        };
        self.0.sharing.lock().unwrap().insert(id, job.clone());
        let inner = self.0.clone();
        let _ = inner
            .events
            .send(SchedulerEvent::SharingChanged { id, failed: false });
        runtime.spawn(async move {
            let result = engine
                .share(request, output, &job.cancel, |mut stats| {
                    stats.uploaded = stats.uploaded.saturating_add(saved_upload);
                    let mut progress = job.progress.lock().unwrap();
                    stats.uploaded = stats
                        .uploaded
                        .max(progress.peers.map_or(0, |stats| stats.uploaded));
                    progress.peers = Some(stats);
                    if !job.ready.swap(true, Ordering::AcqRel) {
                        let _ = inner
                            .events
                            .send(SchedulerEvent::SharingChanged { id, failed: false });
                    }
                })
                .await;
            let mut progress = job.snapshot();
            if let Some(peers) = &mut progress.peers {
                peers.inactive();
            }
            let saved = inner.store.save_progress(id, &progress);
            let failed = (!job.cancel.is_cancelled() && result.is_err()) || saved.is_err();
            inner.sharing.lock().unwrap().remove(&id);
            let _ = done_tx.send(true);
            let _ = inner
                .events
                .send(SchedulerEvent::SharingChanged { id, failed });
        });
        Ok(())
    }

    pub async fn stop_sharing(&self, id: TaskId) {
        let job = self.0.sharing.lock().unwrap().get(&id).cloned();
        if let Some(job) = job {
            job.cancel.cancel();
            let _ = self
                .0
                .events
                .send(SchedulerEvent::SharingChanged { id, failed: false });
            job.stop().await;
        }
    }
}
