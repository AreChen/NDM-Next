use super::*;
use gpui_kit::component::Disableable;

impl Desktop {
    pub(super) fn start_update_watcher(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let mut next_check = Instant::now();
            loop {
                if view
                    .update(cx, |this, cx| {
                        this.try_install_update(cx);
                        if this.preferences.auto_update && Instant::now() >= next_check {
                            this.check_for_update(cx);
                            next_check = Instant::now() + Duration::from_secs(6 * 60 * 60);
                        }
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
            }
        })
        .detach();
    }

    pub(super) fn check_for_update(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.update_state,
            UpdateState::Checking
                | UpdateState::Downloading
                | UpdateState::Ready(_)
                | UpdateState::Installing
        ) {
            return;
        }
        self.update_state = UpdateState::Checking;
        cx.notify();
        let runtime = self.runtime.clone();
        let proxy = self.preferences.proxy.clone();
        cx.spawn(async move |view, cx| {
            let outcome = runtime.spawn(updates::check(proxy)).await;
            let _ = view.update(cx, |this, cx| {
                this.update_state = match outcome {
                    Ok(Ok(Some(release))) => UpdateState::Available(release),
                    Ok(Ok(None)) => UpdateState::Current,
                    _ => UpdateState::Failed,
                };
                if this.preferences.auto_update
                    && matches!(this.update_state, UpdateState::Available(_))
                {
                    this.download_update(false, cx);
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
        self.update_state = UpdateState::Downloading;
        let directory = self
            .preferences_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("updates");
        let proxy = self.preferences.proxy.clone();
        let runtime = self.runtime.clone();
        cx.notify();
        cx.spawn(async move |view, cx| {
            let outcome = runtime
                .spawn(updates::download(release, directory, proxy))
                .await;
            let _ = view.update(cx, |this, cx| {
                this.update_state = match outcome {
                    Ok(Ok(ready)) => UpdateState::Ready(ready),
                    _ => UpdateState::Failed,
                };
                this.try_install_update(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn try_install_update(&mut self, cx: &mut Context<Self>) {
        if !(self.preferences.auto_update || self.install_update_when_idle)
            || self.new_download_open
            || self.editing_url.is_some()
            || self.confirming_host_key.is_some()
            || self.delete_dialog.is_some()
            || self.deleting
            || self.inspecting_hls
        {
            return;
        }
        let UpdateState::Ready(ready) = &self.update_state else {
            return;
        };
        let Ok(tasks) = self.store.list() else {
            return;
        };
        // Consult storage and live workers immediately before launching, including
        // paused workers that have not finished checkpointing yet.
        if tasks.iter().any(|task| {
            matches!(task.state, TaskState::Queued | TaskState::Running)
                || self.scheduler.progress(task.id).is_some()
        }) {
            return;
        }
        let ready = ready.clone();
        self.update_state = UpdateState::Installing;
        self.install_update_when_idle = false;
        let runtime = self.runtime.clone();
        cx.notify();
        cx.spawn(async move |view, cx| {
            // The installer requests the existing graceful shutdown path. Its
            // process survives this app exiting, and relaunches the installed app.
            let outcome = runtime.spawn(updates::install(ready)).await;
            let _ = view.update(cx, |this, cx| {
                this.update_state = if matches!(outcome, Ok(Ok(()))) {
                    UpdateState::Current
                } else {
                    UpdateState::Failed
                };
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn update_status(&self) -> &'static str {
        match self.update_state {
            UpdateState::NotChecked => self.text("尚未检查更新", "Updates not checked"),
            UpdateState::Checking => self.text("正在检查…", "Checking…"),
            UpdateState::Current => self.text("已是最新版本", "You're up to date"),
            UpdateState::Available(_) => self.text("发现新版本", "New version available"),
            UpdateState::Downloading => self.text(
                "正在后台下载并校验更新…",
                "Downloading and verifying update…",
            ),
            UpdateState::Ready(_) => self.text(
                "已校验，空闲后可安装",
                "Verified; ready to install when idle",
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
        let button = Button::new("update-action").small();
        match self.update_state {
            UpdateState::Available(_) => button
                .label(self.text("下载并安装", "Download and install"))
                .on_click(cx.listener(|this, _, _, cx| this.download_update(true, cx))),
            UpdateState::Ready(_) => button
                .label(self.text("安装并重启", "Install and restart"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.install_update_when_idle = true;
                    this.try_install_update(cx);
                })),
            _ => button
                .label(self.text("检查更新", "Check updates"))
                .disabled(matches!(
                    self.update_state,
                    UpdateState::Checking | UpdateState::Downloading | UpdateState::Installing
                ))
                .on_click(cx.listener(|this, _, _, cx| this.check_for_update(cx))),
        }
    }

    pub(super) fn update_banner(
        &self,
        border: u32,
        side: u32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let release = match &self.update_state {
            UpdateState::Available(release) => Some(release),
            UpdateState::Ready(ready) => Some(&ready.release),
            _ => None,
        };
        let mut banner = div()
            .rounded_lg()
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(side))
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .text_sm()
            .child(div().flex_1().child(match release {
                Some(release) => format!("{} · {}", self.update_status(), release.version),
                None => self.update_status().into(),
            }));
        if let Some(release) = release {
            let url = updates::release_url(&release.version);
            banner = banner.child(
                Button::new("update-release-notes")
                    .small()
                    .ghost()
                    .icon(IconName::ExternalLink)
                    .tooltip(self.text("发行说明", "Release notes"))
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
            );
        }
        banner.child(self.update_button(cx))
    }
}
