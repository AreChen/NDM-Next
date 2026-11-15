use super::*;
use gpui_kit::component::Disableable;

impl Desktop {
    pub(super) fn start_update_watcher(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(30))
                .await;
            loop {
                if view
                    .update(cx, |this, cx| {
                        this.try_install_update(cx);
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if this.preferences.auto_update
                            && updates::automatic_check_due(this.preferences.last_update_check, now)
                        {
                            this.check_for_update(false, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_secs(60))
                    .await;
            }
        })
        .detach();
    }

    pub(super) fn check_for_update(&mut self, requested: bool, cx: &mut Context<Self>) {
        if matches!(
            self.update_state,
            UpdateState::Checking
                | UpdateState::Downloading
                | UpdateState::Ready(_)
                | UpdateState::Installing
        ) {
            return;
        }
        self.update_cache_lease = None;
        self.update_state = UpdateState::Checking;
        self.install_update_when_idle = requested;
        self.preferences.last_update_check = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        self.save_preferences(cx);
        cx.notify();
        let runtime = self.runtime.clone();
        let proxy = self.preferences.default_proxy();
        cx.spawn(async move |view, cx| {
            let outcome = runtime.spawn(updates::check(proxy)).await;
            let _ = view.update(cx, |this, cx| {
                this.update_state = match outcome {
                    Ok(Ok(Some(release))) => UpdateState::Available(release),
                    Ok(Ok(None)) => UpdateState::Current,
                    _ => UpdateState::Failed,
                };
                match &this.update_state {
                    UpdateState::Available(release) => {
                        this.preferences.available_update = Some(release.version.to_string())
                    }
                    UpdateState::Current => this.preferences.available_update = None,
                    _ => {}
                }
                this.save_preferences(cx);
                if requested && matches!(this.update_state, UpdateState::Available(_)) {
                    this.download_update(true, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn download_update(&mut self, requested: bool, cx: &mut Context<Self>) {
        let UpdateState::Available(release) = &self.update_state else {
            return;
        };
        let release = release.clone();
        self.install_update_when_idle = requested;
        self.install_wait = None;
        self.update_state = UpdateState::Downloading;
        let directory = self.preferences.cache_dir.join("updates");
        let cache_gate = self.update_cache_gate.clone();
        let proxy = self.preferences.default_proxy();
        let runtime = self.runtime.clone();
        cx.notify();
        cx.spawn(async move |view, cx| {
            let outcome = runtime
                .spawn(async move {
                    let lease = cache_gate.lock_owned().await;
                    updates::download(release, directory, proxy)
                        .await
                        .map(|ready| (ready, lease))
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.update_state = match outcome {
                    Ok(Ok((ready, lease))) => {
                        this.update_cache_lease = Some(lease);
                        UpdateState::Ready(ready)
                    }
                    _ => UpdateState::Failed,
                };
                this.try_install_update(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn try_install_update(&mut self, cx: &mut Context<Self>) {
        if !self
            .update_state
            .installation_requested(self.install_update_when_idle)
        {
            return;
        }
        // Handle validity is authoritative even if a close notification was missed.
        self.delete_dialog = self
            .delete_dialog
            .filter(|handle| handle.update(cx, |_, _, _| ()).is_ok());
        let blocker = if self.new_download_open || self.editing_url.is_some() {
            Some(InstallWait::Editor)
        } else if self.confirming_host_key.is_some() || self.delete_dialog.is_some() {
            Some(InstallWait::Confirmation)
        } else if self.deleting {
            Some(InstallWait::Deleting)
        } else if self.inspecting_hls {
            Some(InstallWait::Media)
        } else {
            None
        };
        if let Some(reason) = blocker {
            self.wait_for_install(reason, cx);
            return;
        }
        let UpdateState::Ready(ready) = &self.update_state else {
            return;
        };
        // Atomically stop new browser commits/resumes before the final idle
        // snapshot. The reservation survives asynchronous hashing and startup.
        let reservation = match self.store.reserve_update_if_idle() {
            Ok(Some(reservation)) => reservation,
            Ok(None) => {
                let downloading = self.store.list().is_ok_and(|tasks| {
                    tasks
                        .iter()
                        .any(|task| matches!(task.state, TaskState::Queued | TaskState::Running))
                });
                self.wait_for_install(
                    if downloading {
                        InstallWait::Downloads
                    } else {
                        InstallWait::OtherInstaller
                    },
                    cx,
                );
                return;
            }
            Err(_) => {
                self.wait_for_install(InstallWait::Storage, cx);
                return;
            }
        };
        let tasks = match self.store.list() {
            Ok(tasks) => tasks,
            Err(_) => {
                self.wait_for_install(InstallWait::Storage, cx);
                return;
            }
        };
        // Consult storage and live workers immediately before launching, including
        // paused workers that have not finished checkpointing yet.
        if tasks.iter().any(|task| {
            matches!(task.state, TaskState::Queued | TaskState::Running)
                || self.scheduler.progress(task.id).is_some()
        }) {
            self.wait_for_install(InstallWait::Downloads, cx);
            return;
        }
        let ready = ready.clone();
        self.update_state = UpdateState::Installing;
        self.install_wait = None;
        let runtime = self.runtime.clone();
        cx.notify();
        cx.spawn(async move |view, cx| {
            // The installer requests the existing graceful shutdown path. Its
            // process survives this app exiting, and relaunches the installed app.
            let outcome = runtime
                .spawn(async move {
                    let _reservation = reservation;
                    updates::install(ready).await
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                if !matches!(outcome, Ok(Ok(()))) {
                    this.update_cache_lease = None;
                }
                this.update_state = if matches!(outcome, Ok(Ok(()))) {
                    this.install_update_when_idle = false;
                    UpdateState::Current
                } else {
                    UpdateState::Failed
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn wait_for_install(&mut self, reason: InstallWait, cx: &mut Context<Self>) {
        if self.install_wait != Some(reason) {
            self.install_wait = Some(reason);
            self.set_notice(self.update_status().to_owned(), cx);
            cx.notify();
        }
    }

    pub(super) fn update_status(&self) -> &'static str {
        if matches!(self.update_state, UpdateState::Ready(_))
            && let Some(reason) = self.install_wait
        {
            return match reason {
                InstallWait::Editor => self.text(
                    "等待关闭新建或编辑下载面板",
                    "Waiting for the download editor to close",
                ),
                InstallWait::Confirmation => {
                    self.text("等待关闭确认窗口", "Waiting for the confirmation to close")
                }
                InstallWait::Deleting => {
                    self.text("删除完成后安装", "Installing after deletion finishes")
                }
                InstallWait::Media => self.text(
                    "媒体解析完成后安装",
                    "Installing after media inspection finishes",
                ),
                InstallWait::Downloads => self.text(
                    "下载结束后安装并重启",
                    "Installing and restarting after downloads finish",
                ),
                InstallWait::OtherInstaller => self.text(
                    "等待其他安装程序退出",
                    "Waiting for another installer to finish",
                ),
                InstallWait::Storage => self.text(
                    "无法读取任务状态，请重试",
                    "Could not read task state; retry installation",
                ),
            };
        }
        match self.update_state {
            UpdateState::NotChecked if self.remembered_update() => self.text(
                "发现新版本，点击升级",
                "New version available; click upgrade",
            ),
            UpdateState::NotChecked => self.text("尚未检查更新", "Updates not checked"),
            UpdateState::Checking => self.text("正在检查…", "Checking…"),
            UpdateState::Current => self.text("已是最新版本", "You're up to date"),
            UpdateState::Available(_) => self.text("发现新版本", "New version available"),
            UpdateState::Downloading => self.text(
                "正在后台下载并校验更新…",
                "Downloading and verifying update…",
            ),
            UpdateState::Ready(_) => self.text(
                "已校验，点击安装并重启",
                "Verified; click to install and restart",
            ),
            UpdateState::Installing => {
                self.text("正在安装，即将重新启动…", "Installing; restarting shortly…")
            }
            UpdateState::Failed => self.text(
                "更新失败，可重试或从 GitHub 下载",
                "Update failed; retry or download from GitHub",
            ),
        }
    }

    pub(super) fn update_button(&self, cx: &mut Context<Self>) -> Button {
        self.update_action("update-action", false, cx)
    }

    fn update_action(&self, id: &'static str, compact: bool, cx: &mut Context<Self>) -> Button {
        let button = Button::new(id).small();
        match self.update_state {
            UpdateState::Available(_) => button
                .label(if compact {
                    self.text("升级", "Upgrade")
                } else {
                    self.text("下载并安装", "Download and install")
                })
                .on_click(cx.listener(|this, _, _, cx| this.download_update(true, cx))),
            UpdateState::Ready(_) => button
                .label(if compact {
                    if self.install_wait.is_some() {
                        self.text("等待升级", "Upgrade pending")
                    } else {
                        self.text("升级", "Upgrade")
                    }
                } else {
                    self.text("安装并重启", "Install and restart")
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.install_update_when_idle = true;
                    this.try_install_update(cx);
                })),
            _ => button
                .label(if compact {
                    match self.update_state {
                        UpdateState::NotChecked | UpdateState::Current => {
                            self.text("升级", "Upgrade")
                        }
                        UpdateState::Checking => self.text("检查升级…", "Checking…"),
                        UpdateState::Downloading => self.text("下载升级…", "Downloading…"),
                        UpdateState::Installing => self.text("正在升级…", "Installing…"),
                        _ => self.text("重试升级", "Retry upgrade"),
                    }
                } else {
                    self.text("检查更新", "Check updates")
                })
                .disabled(matches!(
                    self.update_state,
                    UpdateState::Checking | UpdateState::Downloading | UpdateState::Installing
                ))
                .on_click(cx.listener(move |this, _, _, cx| this.check_for_update(compact, cx))),
        }
    }

    pub(super) fn title_update_button(&self, cx: &mut Context<Self>) -> Button {
        let version = match &self.update_state {
            UpdateState::Available(release) => Some(release.version.clone()),
            UpdateState::Ready(ready) => Some(ready.release.version.clone()),
            _ => updates::remembered_version(self.preferences.available_update.as_deref()),
        };
        let tooltip = match version {
            Some(version) => format!("{} · {}", self.update_status(), version),
            None => self.update_status().to_owned(),
        };
        self.update_action("title-update-action", true, cx)
            .h(px(24.))
            .primary()
            .icon(IconName::ArrowUp)
            .tooltip(tooltip)
    }

    pub(super) fn remembered_update(&self) -> bool {
        updates::remembered_version(self.preferences.available_update.as_deref()).is_some()
    }
}
