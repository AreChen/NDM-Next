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
                    Ok(Err(error)) => UpdateState::Failed(error),
                    _ => UpdateState::Failed(updates::UpdateFailure::Check(None)),
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
        let transfer = updates::UpdateTransfer::new(release.bytes);
        self.update_transfer = Some(transfer.clone());
        self.update_speed.reset();
        self.update_speed.sample(std::time::Instant::now(), 0);
        self.update_bytes_per_second = 0;
        let connections = self.preferences.max_connections;
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
                    updates::download(release, directory, proxy, connections, transfer)
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
                    Ok(Err(error)) => UpdateState::Failed(error),
                    _ => UpdateState::Failed(updates::UpdateFailure::Download),
                };
                this.try_install_update(cx);
                cx.notify();
            });
        })
        .detach();
        self.watch_update_progress(cx);
    }

    fn watch_update_progress(&self, cx: &mut Context<Self>) {
        let token = self
            .update_transfer
            .as_ref()
            .map(|transfer| transfer.verifying.clone());
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let keep_polling = view
                    .update(cx, |this, cx| {
                        if !matches!(this.update_state, UpdateState::Downloading)
                            || !this
                                .update_transfer
                                .as_ref()
                                .zip(token.as_ref())
                                .is_some_and(|(transfer, token)| {
                                    Arc::ptr_eq(&transfer.verifying, token)
                                })
                        {
                            return false;
                        }
                        if let Some(transfer) = &this.update_transfer {
                            this.update_bytes_per_second = this.update_speed.sample(
                                std::time::Instant::now(),
                                transfer.progress.snapshot().transferred,
                            );
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    fn update_download_label(&self, compact: bool) -> String {
        let Some(transfer) = &self.update_transfer else {
            return self
                .text("连接更新服务器…", "Connecting to update server…")
                .into();
        };
        if transfer.is_verifying() {
            return self.text("正在校验 SHA-256…", "Verifying SHA-256…").into();
        }
        let snapshot = transfer.progress.snapshot();
        if snapshot.phase == ndm_core::ProgressPhase::Merging {
            return self.text("正在合并更新…", "Merging update…").into();
        }
        let downloaded = snapshot.downloaded.min(transfer.expected);
        let percent = downloaded as f64 * 100.0 / transfer.expected.max(1) as f64;
        let speed = format!("{}/s", format_bytes(self.update_bytes_per_second));
        if compact {
            format!("{percent:.0}% · {speed}")
        } else {
            format!(
                "{} · {} / {} · {speed}",
                self.text("下载更新", "Downloading update"),
                format_bytes(downloaded),
                format_bytes(transfer.expected)
            )
        }
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let manual_release = ready.release.clone();
        self.update_state = UpdateState::Installing;
        self.install_wait = None;
        let runtime = self.runtime.clone();
        let data_dir = self
            .preferences_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_owned();
        cx.notify();
        cx.spawn(async move |view, cx| {
            // The installer requests the existing graceful shutdown path. Its
            // process survives this app exiting, and relaunches the installed app.
            let outcome = runtime
                .spawn(async move {
                    let _reservation = reservation;
                    updates::install(ready, data_dir).await
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                if !matches!(outcome, Ok(Ok(()))) {
                    this.update_cache_lease = None;
                }
                this.update_state = if matches!(outcome, Ok(Ok(()))) {
                    this.install_update_when_idle = false;
                    #[cfg(target_os = "macos")]
                    {
                        this.set_notice(this.text(
                            "更新磁盘镜像已打开。退出 NDM Next，然后将新版拖入 Applications 替换旧版。",
                            "Update disk image opened. Quit NDM Next, then drag the new app into Applications to replace it.",
                        ).into(), cx);
                        UpdateState::Available(manual_release)
                    }
                    #[cfg(target_os = "linux")]
                    {
                        this.set_notice(this.text(
                            "新版压缩包已校验。退出 NDM Next，解压后运行新版。",
                            "Update archive verified. Quit NDM Next, extract the archive, and run the new version.",
                        ).into(), cx);
                        UpdateState::Available(manual_release)
                    }
                    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                    {
                        // The installer owns its guard; exit so replacement can finish.
                        cx.quit();
                        UpdateState::Current
                    }
                } else {
                    match outcome {
                        Ok(Err(error)) => UpdateState::Failed(error),
                        _ => UpdateState::Failed(updates::UpdateFailure::Launch(None)),
                    }
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

    pub(super) fn update_status(&self) -> String {
        if matches!(self.update_state, UpdateState::Downloading) {
            return self.update_download_label(false);
        }
        if let UpdateState::Failed(error) = &self.update_state {
            return match error {
                updates::UpdateFailure::Check(status) => format!(
                    "{}{}",
                    self.text("检查更新失败，请重试", "Could not check updates; retry"),
                    status
                        .map(|code| format!(" (HTTP {code})"))
                        .unwrap_or_default()
                ),
                updates::UpdateFailure::Download => self
                    .text(
                        "更新下载失败，请检查网络或代理后重试",
                        "Update download failed; check network or proxy and retry",
                    )
                    .into(),
                updates::UpdateFailure::Storage => self
                    .text(
                        "无法写入更新缓存，请检查磁盘空间和目录权限",
                        "Could not write update cache; check disk space and directory permissions",
                    )
                    .into(),
                updates::UpdateFailure::Integrity => self
                    .text(
                        "安装包校验失败，请重新下载",
                        "Installer verification failed; download again",
                    )
                    .into(),
                updates::UpdateFailure::Launch(code) => format!(
                    "{}{}",
                    self.text("无法启动安装程序", "Could not start installer"),
                    code.map(|code| format!(" (OS {code})")).unwrap_or_default()
                ),
                updates::UpdateFailure::Installer(Some(4)) if cfg!(windows) => self
                    .text(
                        "旧程序未能安全退出，请退出后重试安装",
                        "App could not shut down safely; quit and retry installation",
                    )
                    .into(),
                updates::UpdateFailure::Installer(code) => format!(
                    "{}{}",
                    self.text(
                        "安装未完成，可重试或打开安装包",
                        "Installation did not finish; retry or open installer"
                    ),
                    code.map(|code| format!(" (exit {code})"))
                        .unwrap_or_default()
                ),
            };
        }

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
            }
            .to_owned();
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
            UpdateState::Failed(_) => self.text(
                "更新失败，可重试或从 GitHub 下载",
                "Update failed; retry or download from GitHub",
            ),
        }
        .to_owned()
    }

    pub(super) fn update_button(&self, cx: &mut Context<Self>) -> Button {
        self.update_action("update-action", false, cx)
    }

    fn update_action(&self, id: &'static str, compact: bool, cx: &mut Context<Self>) -> Button {
        let button = Button::new(id).small();
        if matches!(self.update_state, UpdateState::Downloading) {
            return button
                .label(self.update_download_label(compact))
                .disabled(true);
        }
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
