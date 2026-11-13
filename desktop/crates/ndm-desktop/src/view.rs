use crate::{
    apply_theme,
    preferences::{Language, Preferences, Theme},
    protocol_auth::{AuthFields, apply as apply_protocol_auth},
    updates::{self, UpdateState},
};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        input::{Input, InputState},
    },
    prelude::*,
    *,
};
use ndm_core::{
    FtpSecurity, HlsCatalog, HlsOptions, NewTask, ProxyOptions, Scheduler, SchedulerEvent,
    SftpOptions, Store, Task, TaskId, TaskOptions, TaskState, inspect_hls_with_options,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    All,
    Active,
    Completed,
    Settings,
}

#[derive(Clone)]
struct PendingHostKey {
    observed: String,
    expected: Option<String>,
}

pub(crate) struct Desktop {
    store: Arc<Store>,
    scheduler: Scheduler,
    runtime: Arc<tokio::runtime::Runtime>,
    preferences: Preferences,
    preferences_path: PathBuf,
    url_input: Entity<InputState>,
    replacement_input: Entity<InputState>,
    directory_input: Entity<InputState>,
    active_input: Entity<InputState>,
    connections_input: Entity<InputState>,
    speed_input: Entity<InputState>,
    proxy_url_input: Entity<InputState>,
    proxy_user_input: Entity<InputState>,
    proxy_password_input: Entity<InputState>,
    auth_user_input: Entity<InputState>,
    auth_password_input: Entity<InputState>,
    auth_key_input: Entity<InputState>,
    auth_passphrase_input: Entity<InputState>,
    auth_expanded: bool,
    ftp_security: FtpSecurity,
    pending_auto_resume: bool,
    pending_host_keys: HashMap<TaskId, PendingHostKey>,
    confirming_host_key: Option<TaskId>,
    editing_url: Option<TaskId>,
    deleting_task: Option<TaskId>,
    hls_catalog: Option<(String, HlsCatalog)>,
    hls_variant: Option<usize>,
    hls_audio: Vec<String>,
    inspecting_hls: bool,
    #[cfg(windows)]
    tray: Option<crate::tray::Tray>,
    #[cfg(windows)]
    startup_enabled: bool,
    section: Section,
    tasks: Vec<Task>,
    sample: HashMap<TaskId, (u64, Instant)>,
    speed: HashMap<TaskId, u64>,
    tracking: bool,
    notice: Option<String>,
    update_state: UpdateState,
}

pub(crate) struct DesktopServices {
    pub store: Arc<Store>,
    pub scheduler: Scheduler,
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub preferences: Preferences,
    pub preferences_path: PathBuf,
    #[cfg(windows)]
    pub ui_sender: tokio::sync::mpsc::UnboundedSender<crate::tray::Command>,
    #[cfg(windows)]
    pub commands: tokio::sync::mpsc::UnboundedReceiver<crate::tray::Command>,
}

impl Desktop {
    pub(crate) fn new(
        services: DesktopServices,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let DesktopServices {
            store,
            scheduler,
            runtime,
            preferences,
            preferences_path,
            #[cfg(windows)]
            ui_sender,
            #[cfg(windows)]
            mut commands,
        } = services;
        let url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        let replacement_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        let directory = preferences.download_dir.to_string_lossy().into_owned();
        let directory_input = cx.new(|cx| InputState::new(window, cx).default_value(directory));
        let active_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences.max_active_tasks.to_string())
        });
        let connections_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences.max_connections.to_string())
        });
        let speed_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences.max_kib_per_second.to_string())
        });
        let proxy_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("http://127.0.0.1:8080")
                .default_value(
                    preferences
                        .proxy
                        .as_ref()
                        .map(|p| p.url.clone())
                        .unwrap_or_default(),
                )
        });
        let proxy_user_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(
                preferences
                    .proxy
                    .as_ref()
                    .and_then(|p| p.username.clone())
                    .unwrap_or_default(),
            )
        });
        let proxy_password_input = cx.new(|cx| {
            InputState::new(window, cx).masked(true).default_value(
                preferences
                    .proxy
                    .as_ref()
                    .and_then(|p| p.password.clone())
                    .unwrap_or_default(),
            )
        });
        let auth_user_input = cx.new(|cx| InputState::new(window, cx));
        let auth_password_input = cx.new(|cx| InputState::new(window, cx).masked(true));
        let auth_key_input = cx.new(|cx| InputState::new(window, cx));
        let auth_passphrase_input = cx.new(|cx| InputState::new(window, cx).masked(true));
        let pending_auto_resume = preferences.auto_resume;
        #[cfg(windows)]
        let startup_enabled = crate::startup::enabled().unwrap_or(false);
        #[cfg(windows)]
        let tray = {
            match crate::tray::Tray::create(ui_sender, preferences.language) {
                Ok(tray) => {
                    window.on_window_should_close(cx, |window, _| {
                        crate::tray::hide(window);
                        false
                    });
                    if std::env::args_os().any(|argument| argument == "--background") {
                        cx.defer_in(window, |_, window, _| crate::tray::hide(window));
                    }
                    Some(tray)
                }
                Err(_) => None,
            }
        };
        #[cfg(windows)]
        {
            let runtime = runtime.clone();
            let scheduler = scheduler.clone();
            cx.spawn_in(window, async move |view, cx| {
                while let Some(command) = commands.recv().await {
                    match command {
                        crate::tray::Command::Show => {
                            if view
                                .update_in(cx, |_, window, _| crate::tray::show(window))
                                .is_err()
                            {
                                break;
                            }
                        }
                        crate::tray::Command::Quit => {
                            let scheduler = scheduler.clone();
                            let _ = runtime
                                .spawn(async move { scheduler.shutdown().await })
                                .await;
                            let _ = view.update_in(cx, |_, _, cx| cx.quit());
                            break;
                        }
                    }
                }
            })
            .detach();
        }
        let tasks = store.list().unwrap_or_default();
        let mut events = scheduler.subscribe();
        cx.spawn(async move |view, cx| {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if view.update(cx, |this, cx| {
                            this.reload();
                            match event {
                                SchedulerEvent::Started(_) => this.track_progress(cx),
                                SchedulerEvent::SftpHostKeyRequired { id, fingerprint } => {
                                    this.pending_host_keys.insert(id, PendingHostKey { observed: fingerprint, expected: None });
                                    this.notice = Some(this.text("SFTP 服务器密钥需要核对。", "Check the SFTP server key before trusting it.").into());
                                },
                                SchedulerEvent::SftpHostKeyChanged { id, expected, observed } => {
                                    this.pending_host_keys.insert(id, PendingHostKey { observed, expected: Some(expected) });
                                    this.notice = Some(this.text("SFTP 服务器密钥已改变，请谨慎核对。", "The SFTP server key has changed; verify it carefully.").into());
                                },
                                SchedulerEvent::Finished(_) => {}
                            }
                            cx.notify();
                        }).is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }).detach();
        let mut this = Self {
            store,
            scheduler,
            runtime,
            preferences,
            preferences_path,
            url_input,
            replacement_input,
            directory_input,
            active_input,
            connections_input,
            speed_input,
            proxy_url_input,
            proxy_user_input,
            proxy_password_input,
            auth_user_input,
            auth_password_input,
            auth_key_input,
            auth_passphrase_input,
            auth_expanded: false,
            ftp_security: FtpSecurity::Auto,
            pending_auto_resume,
            pending_host_keys: HashMap::new(),
            confirming_host_key: None,
            editing_url: None,
            deleting_task: None,
            hls_catalog: None,
            hls_variant: None,
            hls_audio: Vec::new(),
            inspecting_hls: false,
            #[cfg(windows)]
            tray,
            #[cfg(windows)]
            startup_enabled,
            section: Section::All,
            tasks,
            sample: HashMap::new(),
            speed: HashMap::new(),
            tracking: false,
            notice: None,
            update_state: if updates::is_store_managed() {
                UpdateState::StoreManaged
            } else {
                UpdateState::NotChecked
            },
        };
        if this
            .tasks
            .iter()
            .any(|task| task.state == TaskState::Running)
        {
            this.track_progress(cx);
        }
        if this.update_state != UpdateState::StoreManaged {
            cx.spawn(async move |view, cx| {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                let _ = view.update(cx, |this, cx| this.check_for_update(cx));
            })
            .detach();
        }
        this
    }

    fn text(&self, chinese: &'static str, english: &'static str) -> &'static str {
        match self.preferences.language {
            Language::Chinese => chinese,
            Language::English => english,
        }
    }
    fn reload(&mut self) {
        if let Ok(tasks) = self.store.list() {
            self.tasks = tasks;
        }
    }
    fn check_for_update(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.update_state,
            UpdateState::Checking | UpdateState::StoreManaged
        ) {
            return;
        }
        self.update_state = UpdateState::Checking;
        cx.notify();
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let outcome = runtime.spawn(updates::check()).await;
            let _ = view.update(cx, |this, cx| {
                this.update_state = match outcome {
                    Ok(Ok(Some(version))) => UpdateState::Available(version),
                    Ok(Ok(None)) => UpdateState::Current,
                    _ => UpdateState::Failed,
                };
                cx.notify();
            });
        })
        .detach();
    }
    fn track_progress(&mut self, cx: &mut Context<Self>) {
        if self.tracking {
            return;
        }
        self.tracking = true;
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep_tracking = view
                    .update(cx, |this, cx| {
                        this.reload();
                        let mut any = false;
                        for task in &this.tasks {
                            if let Some(progress) = this.scheduler.progress(task.id) {
                                any = true;
                                let now = Instant::now();
                                if let Some((old, then)) =
                                    this.sample.insert(task.id, (progress.downloaded, now))
                                {
                                    let elapsed = now.duration_since(then).as_secs_f64().max(0.001);
                                    this.speed.insert(
                                        task.id,
                                        (progress.downloaded.saturating_sub(old) as f64 / elapsed)
                                            as u64,
                                    );
                                }
                            }
                        }
                        this.tracking = any;
                        cx.notify();
                        any
                    })
                    .unwrap_or(false);
                if !keep_tracking {
                    break;
                }
            }
        })
        .detach();
    }
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self
            .url_input
            .read(cx)
            .value()
            .to_string()
            .trim()
            .to_owned();
        let destination = self.directory_input.read(cx).value().to_string();
        let mut parsed = match reqwest::Url::parse(&url) {
            Ok(url) if matches!(url.scheme(), "http" | "https" | "ftp" | "ftps" | "sftp") => url,
            _ => {
                self.notice = Some(
                    self.text("请输入有效的下载链接。", "Enter a valid download URL.")
                        .into(),
                );
                cx.notify();
                return;
            }
        };
        let auth = AuthFields {
            username: self.auth_user_input.read(cx).value().to_string(),
            password: self.auth_password_input.read(cx).value().to_string(),
            key_path: self.auth_key_input.read(cx).value().to_string(),
            passphrase: self.auth_passphrase_input.read(cx).value().to_string(),
        };
        let protocol_options = match apply_protocol_auth(&mut parsed, &auth, self.ftp_security) {
            Ok(options) => options,
            Err(_) => {
                self.notice = Some(
                    self.text(
                        "请检查 FTP/SFTP 凭据与连接方式。",
                        "Check the FTP/SFTP credentials and connection mode.",
                    )
                    .into(),
                );
                cx.notify();
                return;
            }
        };
        if destination.trim().is_empty() {
            self.notice = Some(
                self.text("请选择保存位置。", "Choose a destination folder.")
                    .into(),
            );
            cx.notify();
            return;
        }
        let filename = parsed
            .path_segments()
            .and_then(|mut segments| segments.rfind(|name| !name.is_empty()))
            .map(str::to_owned);
        let request = NewTask {
            url: parsed.to_string(),
            destination: PathBuf::from(destination),
            suggested_filename: filename,
            source_url: None,
            expected_size: None,
            headers: BTreeMap::new(),
        };
        let id = TaskId::new_v4();
        let options = if matches!(protocol_options, TaskOptions::Auto)
            && self
                .hls_catalog
                .as_ref()
                .is_some_and(|(catalog_url, _)| catalog_url == &url)
        {
            TaskOptions::Hls(HlsOptions {
                variant_index: self.hls_variant,
                audio_names: self.hls_audio.clone(),
                ..Default::default()
            })
        } else {
            protocol_options
        };
        match self
            .store
            .prepare_with_options(id, &request, &options)
            .and_then(|_| self.store.commit(id))
        {
            Ok(_) => {
                self.scheduler.notify();
                self.preferences.download_dir = request.destination.clone();
                self.save_preferences();
                self.notice = Some(self.text("下载已添加。", "Download added.").into());
                self.section = Section::All;
                self.hls_catalog = None;
                self.hls_variant = None;
                self.hls_audio.clear();
                self.ftp_security = FtpSecurity::Auto;
                self.auth_expanded = false;
                for input in [
                    &self.auth_user_input,
                    &self.auth_password_input,
                    &self.auth_key_input,
                    &self.auth_passphrase_input,
                ] {
                    input.update(cx, |state, cx| state.set_value("", window, cx));
                }
                self.reload();
            }
            Err(_) => {
                self.notice = Some(
                    self.text("无法添加下载任务。", "Could not add the download.")
                        .into(),
                )
            }
        }
        cx.notify();
    }
    fn inspect_hls(&mut self, cx: &mut Context<Self>) {
        if self.inspecting_hls {
            return;
        }
        let url = self
            .url_input
            .read(cx)
            .value()
            .to_string()
            .trim()
            .to_owned();
        if !reqwest::Url::parse(&url).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
            self.notice = Some(
                self.text("请输入有效的 HLS 链接。", "Enter a valid HLS URL.")
                    .into(),
            );
            cx.notify();
            return;
        }
        let request = NewTask {
            url: url.clone(),
            destination: self.preferences.download_dir.clone(),
            suggested_filename: None,
            source_url: None,
            expected_size: None,
            headers: BTreeMap::new(),
        };
        let options = HlsOptions {
            proxy: self.preferences.proxy.clone(),
            ..Default::default()
        };
        self.inspecting_hls = true;
        self.notice = Some(
            self.text("正在读取媒体选项…", "Inspecting media options…")
                .into(),
        );
        let result = self.runtime.spawn(async move {
            inspect_hls_with_options(
                &request,
                &options,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
        });
        cx.spawn(async move |view, cx| {
            let result = result.await;
            let _ = view.update(cx, |this, cx| {
                this.inspecting_hls = false;
                if this.url_input.read(cx).value().as_ref().trim() == url {
                    match result {
                        Ok(Ok(catalog)) => {
                            this.hls_catalog = Some((url, catalog));
                            this.hls_variant = None;
                            this.hls_audio.clear();
                            this.notice = Some(
                                this.text(
                                    "请选择清晰度和音轨，或使用默认选项。",
                                    "Choose quality and audio tracks, or use the defaults.",
                                )
                                .into(),
                            );
                        }
                        _ => {
                            this.hls_catalog = None;
                            this.notice = Some(
                                this.text(
                                    "无法读取 HLS 列表，请检查链接或代理。",
                                    "Could not inspect the HLS playlist; check the URL or proxy.",
                                )
                                .into(),
                            );
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save_preferences(&mut self) {
        if self.preferences.save(&self.preferences_path).is_err() {
            self.notice = Some(
                self.text("无法保存设置。", "Could not save settings.")
                    .into(),
            );
        }
    }
    fn set_theme(&mut self, mode: Theme, cx: &mut Context<Self>) {
        self.preferences.theme = mode;
        apply_theme(mode, cx);
        self.save_preferences();
        cx.notify();
    }
    fn save_download_settings(&mut self, cx: &mut Context<Self>) {
        let active = self.active_input.read(cx).value().parse::<usize>();
        let connections = self.connections_input.read(cx).value().parse::<usize>();
        let speed = self.speed_input.read(cx).value().parse::<u64>();
        let (Ok(active), Ok(connections), Ok(speed)) = (active, connections, speed) else {
            self.notice = Some(
                self.text("请输入有效的数字设置。", "Enter valid numeric settings.")
                    .into(),
            );
            cx.notify();
            return;
        };
        if !(1..=32).contains(&active) || !(1..=16).contains(&connections) || speed > 1_000_000 {
            self.notice = Some(
                self.text(
                    "任务数须为 1–32，连接数须为 1–16，限速须为 0–1000000。",
                    "Tasks must be 1–32, connections 1–16, and speed 0–1000000.",
                )
                .into(),
            );
            cx.notify();
            return;
        }
        let url = self
            .proxy_url_input
            .read(cx)
            .value()
            .to_string()
            .trim()
            .to_owned();
        let username = self.proxy_user_input.read(cx).value().to_string();
        let password = self.proxy_password_input.read(cx).value().to_string();
        let proxy = if url.is_empty() {
            if !username.is_empty() || !password.is_empty() {
                self.notice = Some(
                    self.text("请先填写代理地址。", "Enter a proxy address first.")
                        .into(),
                );
                cx.notify();
                return;
            }
            None
        } else {
            let config = ProxyOptions {
                url,
                username: (!username.is_empty()).then_some(username),
                password: (!password.is_empty()).then_some(password),
            };
            if config.validate().is_err() {
                self.notice = Some(
                    self.text(
                        "代理地址须为 HTTP(S)，用户名和密码请单独填写。",
                        "Use an HTTP(S) proxy URL and enter credentials separately.",
                    )
                    .into(),
                );
                cx.notify();
                return;
            }
            Some(config)
        };
        let mut updated = self.preferences.clone();
        updated.max_active_tasks = active;
        updated.max_connections = connections;
        updated.max_kib_per_second = speed;
        updated.auto_resume = self.pending_auto_resume;
        updated.proxy = proxy;
        if updated.save(&self.preferences_path).is_err() {
            self.notice = Some(
                self.text("无法保存设置。", "Could not save settings.")
                    .into(),
            );
        } else {
            self.scheduler.update_options(updated.scheduler_options());
            self.preferences = updated;
            self.notice = Some(
                self.text(
                    "设置已保存；正在运行的任务会保持原设置。",
                    "Settings saved; running downloads keep their current settings.",
                )
                .into(),
            );
        }
        cx.notify();
    }
    #[cfg(windows)]
    fn toggle_startup(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.startup_enabled;
        match crate::startup::set(enabled) {
            Ok(()) => {
                self.startup_enabled = enabled;
                self.notice = Some(
                    self.text("开机自启设置已更新。", "Startup setting updated.")
                        .into(),
                );
            }
            Err(_) => {
                self.notice = Some(
                    self.text(
                        "无法更新开机自启，请检查系统设置或其他安装。",
                        "Could not update startup; check system settings or another installation.",
                    )
                    .into(),
                )
            }
        }
        cx.notify();
    }
    fn task_action(&mut self, id: TaskId, state: TaskState) {
        let scheduler = self.scheduler.clone();
        self.runtime.spawn(async move {
            match state {
                TaskState::Queued | TaskState::Running => {
                    let _ = scheduler.pause(id).await;
                }
                TaskState::Paused | TaskState::Failed => {
                    let _ = scheduler.resume(id).await;
                }
                _ => {}
            }
        });
    }
    fn trust_host_key(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let Some(key) = self.pending_host_keys.get(&id).cloned() else {
            return;
        };
        let Ok(Some(task)) = self.store.get(id) else {
            return;
        };
        let mut options = match task.options {
            TaskOptions::Auto => SftpOptions::default(),
            TaskOptions::Sftp(options) => options,
            _ => return,
        };
        options.trusted_fingerprint = Some(key.observed);
        match self.store.set_options(id, &TaskOptions::Sftp(options)) {
            Ok(()) => {
                self.pending_host_keys.remove(&id);
                self.confirming_host_key = None;
                self.task_action(id, TaskState::Failed);
                self.notice = Some(
                    self.text(
                        "服务器密钥已保存，正在重新连接。",
                        "Server key saved; reconnecting.",
                    )
                    .into(),
                );
            }
            Err(_) => {
                self.notice = Some(
                    self.text("无法保存服务器密钥。", "Could not save the server key.")
                        .into(),
                )
            }
        }
        cx.notify();
    }
    fn delete_record(&mut self, id: TaskId, remove_partial: bool, cx: &mut Context<Self>) {
        match self.scheduler.delete(id, remove_partial) {
            Ok(()) => {
                self.pending_host_keys.remove(&id);
                self.deleting_task = None;
                if self.editing_url == Some(id) {
                    self.editing_url = None;
                }
                self.reload();
                self.notice = Some(if remove_partial {
                    self.text("记录和临时文件已移除。", "Record and temporary files removed.")
                } else {
                    self.text("记录已移除，文件已保留。", "Record removed; files were kept.")
                }.into());
            }
            Err(_) => {
                self.notice = Some(self.text(
                    "无法移除任务，请先暂停并检查临时目录和文件权限。",
                    "Could not remove the task. Pause it and check the temporary folder and file permissions.",
                ).into())
            }
        }
        cx.notify();
    }
    fn save_replacement_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.editing_url else { return };
        let url = self.replacement_input.read(cx).value().to_string();
        match self.store.update_web_url(id, url.trim()) {
            Ok(()) => {
                self.editing_url = None;
                self.replacement_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.reload();
                self.notice = Some(
                    self.text(
                        "链接已更新，可以继续下载。",
                        "Link updated; you can resume the download.",
                    )
                    .into(),
                );
            }
            Err(_) => {
                self.notice = Some(
                    self.text(
                        "链接无效，或任务已不再暂停。",
                        "The link is invalid or the task is no longer paused.",
                    )
                    .into(),
                );
            }
        }
        cx.notify();
    }
    fn is_dark(&self, cx: &App) -> bool {
        match self.preferences.theme {
            Theme::Dark => true,
            Theme::Light => false,
            Theme::System => matches!(
                cx.window_appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

impl Desktop {
    fn settings_field(
        &self,
        label: &'static str,
        id: &'static str,
        input: &Entity<InputState>,
        muted: u32,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().text_color(rgb(muted)).child(label))
            .child(Input::new(input).id(id).aria_label(label).w_full())
    }
    fn hls_options_panel(
        &self,
        catalog: &HlsCatalog,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut panel = div()
            .id("hls-options")
            .max_h(px(200.))
            .overflow_y_scroll()
            .rounded_xl()
            .border_1()
            .border_color(rgb(if dark { 0x28344b } else { 0xe3e8f2 }))
            .bg(rgb(if dark { 0x111625 } else { 0xf8f9fd }))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.text("媒体选项", "Media options")),
            );
        if catalog.direct_media {
            return panel.child(self.text(
                "媒体列表无需选择清晰度。",
                "No quality selection is needed for this media playlist.",
            ));
        }
        let selected = self.hls_variant.or_else(|| {
            catalog
                .variants
                .iter()
                .max_by_key(|v| v.bandwidth)
                .map(|v| v.index)
        });
        let mut qualities = div().flex().flex_wrap().gap_2();
        for variant in &catalog.variants {
            let id = variant.index;
            let quality = variant
                .resolution
                .map(|(w, h)| format!("{w} × {h}"))
                .unwrap_or_else(|| self.text("自动尺寸", "Auto size").into());
            let label = format!(
                "{quality} · {:.1} Mbps",
                variant.bandwidth as f64 / 1_000_000.
            );
            let button = Button::new(SharedString::from(format!("hls-variant-{id}")))
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.hls_variant = Some(id);
                    this.hls_audio.clear();
                    cx.notify();
                }));
            qualities = qualities.child(if selected == Some(id) {
                button.primary()
            } else {
                button
            });
        }
        panel = panel.child(qualities);
        if let Some(variant) = catalog
            .variants
            .iter()
            .find(|variant| Some(variant.index) == selected)
            && !variant.audio_tracks.is_empty()
        {
            let mut tracks = div().flex().flex_wrap().gap_2();
            for track in &variant.audio_tracks {
                let name = track.name.clone();
                let chosen = self.hls_audio.contains(&name);
                let label = match &track.language {
                    Some(language) => format!("{} ({language})", track.name),
                    None => track.name.clone(),
                };
                let button = Button::new(SharedString::from(format!("hls-audio-{name}")))
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(index) = this.hls_audio.iter().position(|item| item == &name) {
                            this.hls_audio.remove(index);
                        } else {
                            this.hls_audio.push(name.clone());
                        }
                        cx.notify();
                    }));
                tracks = tracks.child(if chosen { button.primary() } else { button });
            }
            panel = panel
                .child(div().text_sm().child(self.text(
                    "音轨可多选，未选择时使用默认音轨。",
                    "Select multiple audio tracks, or leave empty for the default.",
                )))
                .child(tracks);
        }
        panel
    }
    fn nav_item(
        &self,
        label: &'static str,
        section: Section,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.section == section;
        let dark = self.is_dark(cx);
        let background = if selected {
            if dark { 0x292b50 } else { 0xeceaff }
        } else if dark {
            0x111625
        } else {
            0xf8f9fd
        };
        let foreground = if selected {
            0x8277ff
        } else if dark {
            0xaab7d0
        } else {
            0x5f6c84
        };
        div()
            .id(("nav", section as usize))
            .cursor_pointer()
            .rounded_lg()
            .px_4()
            .py_3()
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.section = section;
                cx.notify();
            }))
            .child(label)
    }
    fn task_row(&self, task: &Task, dark: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let name = task
            .request
            .suggested_filename
            .clone()
            .unwrap_or_else(|| self.text("未命名下载", "Untitled download").into());
        let status = match task.state {
            TaskState::Prepared => self.text("等待确认", "Awaiting confirmation"),
            TaskState::Queued => self.text("等待中", "Queued"),
            TaskState::Running => self.text("下载中", "Downloading"),
            TaskState::Paused => self.text("已暂停", "Paused"),
            TaskState::Completed => self.text("已完成", "Completed"),
            TaskState::Failed => self.text("失败", "Failed"),
        };
        let mut detail = match self.scheduler.progress(task.id) {
            Some(progress) => {
                let size = match progress.total {
                    Some(total) => format!(
                        "{} / {}",
                        format_bytes(progress.downloaded),
                        format_bytes(total)
                    ),
                    None => format_bytes(progress.downloaded),
                };
                format!(
                    "{size}   ·   {}/s",
                    format_bytes(self.speed.get(&task.id).copied().unwrap_or(0))
                )
            }
            None => match task.error.as_deref() {
                Some("hls_recovery_conflict") => self.text(
                    "无法安全续传：媒体或临时文件已改变，已保留临时数据。",
                    "Cannot resume safely: media or temporary data changed. The temporary data was kept.",
                ).into(),
                Some("hls_failed") => self.text(
                    "无法下载或合并媒体，请检查链接和网络。",
                    "Could not download or merge the media. Check the URL and network.",
                ).into(),
                Some(error) => error.to_owned(),
                None => status.into(),
            },
        };
        let id = task.id;
        let state = task.state;
        let is_hls = matches!(&task.options, TaskOptions::Hls(_))
            || (matches!(&task.options, TaskOptions::Auto)
                && reqwest::Url::parse(&task.request.url)
                    .is_ok_and(|url| url.path().to_ascii_lowercase().ends_with(".m3u8")));
        let can_edit_url = matches!(state, TaskState::Paused | TaskState::Failed)
            && !matches!(&task.options, TaskOptions::Ftp(_) | TaskOptions::Sftp(_))
            && reqwest::Url::parse(&task.request.url)
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
        let pending_key = self.pending_host_keys.get(&id);
        if let Some(key) = pending_key {
            let host = reqwest::Url::parse(&task.request.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_default();
            detail = if let Some(expected) = &key.expected {
                format!(
                    "{} {host}\n{} {expected}\n{} {}",
                    self.text("密钥已改变：", "Key changed:"),
                    self.text("原指纹：", "Previous:"),
                    self.text("新指纹：", "New:"),
                    key.observed
                )
            } else {
                format!(
                    "{} {host}\n{} {}",
                    self.text("首次连接：", "First connection:"),
                    self.text("指纹：", "Fingerprint:"),
                    key.observed
                )
            };
        }
        let action = match state {
            TaskState::Queued | TaskState::Running => Some(self.text("暂停", "Pause")),
            TaskState::Paused | TaskState::Failed => Some(self.text("继续", "Resume")),
            _ => None,
        };
        let fg = if dark { 0xecf0ff } else { 0x1f2b42 };
        let muted = if dark { 0x8897b3 } else { 0x72809b };
        let card = if dark { 0x171f31 } else { 0xffffff };
        let border = if dark { 0x28344b } else { 0xe6eaf2 };
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_4()
            .p_4()
            .rounded_xl()
            .bg(rgb(card))
            .border_1()
            .border_color(rgb(border))
            .child(
                div()
                    .w(px(42.))
                    .h(px(42.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_lg()
                    .bg(rgb(if dark { 0x292b52 } else { 0xeeecff }))
                    .text_color(rgb(0x8d82ff))
                    .child("↓"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_color(rgb(fg))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_sm()
                            .child(name),
                    )
                    .child(div().text_color(rgb(muted)).text_xs().child(detail)),
            )
            .child(
                div()
                    .text_color(rgb(if state == TaskState::Completed {
                        0x39bc8d
                    } else {
                        muted
                    }))
                    .text_xs()
                    .child(status),
            )
            .when_some(action, |row, action| {
                row.child(
                    Button::new(SharedString::from(format!("action-{id}")))
                        .label(action)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.task_action(id, state);
                            cx.notify();
                        })),
                )
            })
            .when(state == TaskState::Running && is_hls, |row| row.child(
                Button::new(SharedString::from(format!("stop-live-{id}"))).label(self.text("停止并保存", "Stop and save")).on_click(cx.listener(move |this, _, _, cx| {
                    if this.scheduler.stop_live(id).is_err() { this.notice = Some(this.text("无法停止该媒体任务。", "Could not stop the media task.").into()); }
                    cx.notify();
                }))
            ))
            .when(can_edit_url, |row| row.child(
                Button::new(SharedString::from(format!("edit-url-{id}")))
                    .label(self.text("更换链接", "Change link"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.replacement_input.update(cx, |input, cx| input.set_value("", window, cx));
                        this.editing_url = Some(id);
                        cx.notify();
                    }))
            ))
            .when(pending_key.is_some(), |row| {
                if self.confirming_host_key == Some(id) {
                    row.child(Button::new(SharedString::from(format!("trust-confirm-{id}"))).label(self.text("确认信任", "Confirm trust")).on_click(cx.listener(move |this, _, _, cx| this.trust_host_key(id, cx))))
                        .child(Button::new(SharedString::from(format!("trust-cancel-{id}"))).label(self.text("取消", "Cancel")).on_click(cx.listener(move |this, _, _, cx| { this.confirming_host_key = None; cx.notify(); })))
                } else {
                    row.child(Button::new(SharedString::from(format!("trust-review-{id}"))).label(self.text("核对密钥", "Review key")).on_click(cx.listener(move |this, _, _, cx| {
                        this.confirming_host_key = Some(id);
                        this.notice = Some(this.text("请通过可信渠道核对服务器指纹；确认后才会连接。", "Verify the fingerprint through a trusted channel before confirming.").into());
                        cx.notify();
                    })))
                }
            })
            .when(
                !matches!(state, TaskState::Running | TaskState::Prepared),
                |row| {
                    row.child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .label(self.text("移除", "Remove"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if state == TaskState::Completed {
                                    this.delete_record(id, false, cx);
                                } else {
                                    this.deleting_task = Some(id);
                                    cx.notify();
                                }
                            })),
                    )
                },
            )
    }
}

impl Render for Desktop {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.is_dark(cx);
        let page = if dark { 0x0c1120 } else { 0xf4f6fb };
        let side = if dark { 0x111625 } else { 0xf8f9fd };
        let fg = if dark { 0xf0f3ff } else { 0x19253b };
        let muted = if dark { 0x8999b4 } else { 0x6e7d98 };
        let border = if dark { 0x28344b } else { 0xe3e8f2 };
        let visible: Vec<_> = self
            .tasks
            .iter()
            .filter(|task| match self.section {
                Section::All => true,
                Section::Active => matches!(
                    task.state,
                    TaskState::Queued | TaskState::Running | TaskState::Paused
                ),
                Section::Completed => task.state == TaskState::Completed,
                Section::Settings => false,
            })
            .cloned()
            .collect();
        let count_active = self
            .tasks
            .iter()
            .filter(|task| task.state == TaskState::Running)
            .count();
        let title = match self.section {
            Section::All => self.text("全部下载", "All downloads"),
            Section::Active => self.text("正在进行", "In progress"),
            Section::Completed => self.text("已完成", "Completed"),
            Section::Settings => self.text("设置", "Settings"),
        };
        let mut content = div()
            .id("content")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_5()
            .p_8();
        content = content.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(fg))
                                .child(title),
                        )
                        .child(div().text_sm().text_color(rgb(muted)).child(format!(
                            "{}  ·  {} {}",
                            self.text("掌控每一次下载", "Downloads, under control"),
                            count_active,
                            self.text("个任务正在运行", "active")
                        ))),
                )
                .child(
                    div()
                        .rounded_full()
                        .px_3()
                        .py_1()
                        .bg(rgb(if dark { 0x183b36 } else { 0xddf7ed }))
                        .text_color(rgb(0x31b98a))
                        .text_xs()
                        .child(self.text("准备就绪", "Ready")),
                ),
        );
        if let UpdateState::Available(version) = &self.update_state {
            let url = updates::release_url(version);
            content = content.child(
                div()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(border))
                    .bg(rgb(side))
                    .p_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(format!(
                        "{} {version}",
                        self.text("发现新版本", "Update available")
                    ))
                    .child(
                        Button::new("open-update")
                            .label(self.text("查看更新", "View release"))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                    ),
            );
        }
        if self.section == Section::Settings {
            content = content.overflow_y_scroll();
            let language = self.preferences.language;
            content = content.child(
                div()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(border))
                    .bg(rgb(side))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_color(rgb(fg))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.text("外观和语言", "Appearance and language")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(muted))
                            .child(self.text("主题", "Theme")),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("theme-system")
                                    .label(self.text("跟随系统", "System"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_theme(Theme::System, cx)
                                    })),
                            )
                            .child(
                                Button::new("theme-light")
                                    .label(self.text("亮色", "Light"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_theme(Theme::Light, cx)
                                    })),
                            )
                            .child(
                                Button::new("theme-dark")
                                    .label(self.text("暗色", "Dark"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_theme(Theme::Dark, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(muted))
                            .child(self.text("语言", "Language")),
                    )
                    .child(
                        Button::new("language")
                            .label(if language == Language::Chinese {
                                "切换至 English"
                            } else {
                                "Switch to 中文"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.language =
                                    if this.preferences.language == Language::Chinese {
                                        Language::English
                                    } else {
                                        Language::Chinese
                                    };
                                #[cfg(windows)]
                                if let Some(tray) = &this.tray {
                                    tray.set_language(this.preferences.language);
                                }
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            );
            content = content.child(
                div()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(border))
                    .bg(rgb(side))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_color(rgb(fg))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.text("下载与网络", "Downloads and network")),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(self.settings_field(
                                self.text("同时下载任务数", "Concurrent downloads"),
                                "max-active",
                                &self.active_input,
                                muted,
                            ))
                            .child(self.settings_field(
                                self.text("每个任务连接数", "Connections per download"),
                                "max-connections",
                                &self.connections_input,
                                muted,
                            ))
                            .child(self.settings_field(
                                self.text(
                                    "每个任务限速 KiB/s（0 不限）",
                                    "Limit per download KiB/s (0 unlimited)",
                                ),
                                "max-speed",
                                &self.speed_input,
                                muted,
                            )),
                    )
                    .child(
                        Button::new("auto-resume")
                            .label(if self.pending_auto_resume {
                                self.text("启动时恢复任务：开启", "Resume on startup: On")
                            } else {
                                self.text("启动时恢复任务：关闭", "Resume on startup: Off")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pending_auto_resume = !this.pending_auto_resume;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_color(rgb(fg))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.text("HTTP(S) 代理", "HTTP(S) proxy")),
                    )
                    .child(div().text_sm().text_color(rgb(muted)).child(self.text(
                        "用于 HTTP 和 HLS；留空时使用系统代理设置。",
                        "Used for HTTP and HLS; leave blank to use system proxy settings.",
                    )))
                    .child(self.settings_field(
                        self.text("代理地址", "Proxy URL"),
                        "proxy-url",
                        &self.proxy_url_input,
                        muted,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(self.settings_field(
                                self.text("用户名（可选）", "Username (optional)"),
                                "proxy-user",
                                &self.proxy_user_input,
                                muted,
                            ))
                            .child(self.settings_field(
                                self.text("密码（可选）", "Password (optional)"),
                                "proxy-password",
                                &self.proxy_password_input,
                                muted,
                            )),
                    )
                    .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                        "代理凭据由 Windows 加密保存在本机。",
                        "Proxy credentials are encrypted by Windows on this device.",
                    )))
                    .child(
                        Button::new("save-download-settings")
                            .primary()
                            .label(self.text("保存设置", "Save settings"))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.save_download_settings(cx)),
                            ),
                    ),
            );
            #[cfg(windows)]
            {
                content = content.child(
                    div()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(border))
                        .bg(rgb(side))
                        .p_6()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(self.text("启动", "Startup")),
                        )
                        .child(div().text_sm().text_color(rgb(muted)).child(self.text(
                            "开机后在托盘中运行，默认关闭。",
                            "Start in the system tray when you sign in; off by default.",
                        )))
                        .child(
                            Button::new("startup-toggle")
                                .label(if self.startup_enabled {
                                    self.text("开机自启：开启", "Start on sign-in: On")
                                } else {
                                    self.text("开机自启：关闭", "Start on sign-in: Off")
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_startup(cx))),
                        ),
                );
            }
            let update_description = match &self.update_state {
                UpdateState::NotChecked => {
                    self.text("尚未检查更新。", "Updates have not been checked.")
                }
                UpdateState::Checking => self.text("正在检查更新…", "Checking for updates…"),
                UpdateState::Current => {
                    self.text("没有发现新版本。", "No newer release was found.")
                }
                UpdateState::Available(_) => self.text(
                    "新版本已发布，可在上方查看。",
                    "A new release is available above.",
                ),
                UpdateState::Failed => self.text(
                    "检查更新失败，请稍后重试。",
                    "Could not check for updates; try again later.",
                ),
                UpdateState::StoreManaged => self.text(
                    "此安装由 Microsoft Store 更新。",
                    "Microsoft Store manages updates for this installation.",
                ),
            };
            let mut update_card = div()
                .rounded_xl()
                .border_1()
                .border_color(rgb(border))
                .bg(rgb(side))
                .p_6()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.text("关于与更新", "About and updates")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(muted))
                        .child(format!("NDM Next {}", env!("CARGO_PKG_VERSION"))),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(muted))
                        .child(update_description),
                );
            if self.update_state != UpdateState::StoreManaged {
                update_card = update_card
                    .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                        "启动后会向 GitHub 检查桌面版更新。",
                        "Checks GitHub for desktop updates after startup.",
                    )))
                    .child(
                        Button::new("check-updates")
                            .label(self.text("检查更新", "Check for updates"))
                            .on_click(cx.listener(|this, _, _, cx| this.check_for_update(cx))),
                    );
            }
            content = content.child(update_card);
        } else {
            content = content.child(
                div()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(border))
                    .bg(rgb(side))
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_color(rgb(fg))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.text("新建下载", "New download")),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div().flex_1().child(
                                    Input::new(&self.url_input)
                                        .id("download-url")
                                        .aria_label(self.text("下载链接", "Download URL"))
                                        .w_full(),
                                ),
                            )
                            .child(
                                Button::new("advanced-auth")
                                    .label(self.text("FTP/SFTP 认证", "FTP/SFTP auth"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.auth_expanded = !this.auth_expanded;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("inspect-hls")
                                    .label(if self.inspecting_hls {
                                        self.text("正在读取…", "Inspecting…")
                                    } else {
                                        self.text("媒体选项", "Media options")
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.inspect_hls(cx))),
                            )
                            .child(
                                Button::new("add-download")
                                    .primary()
                                    .label(self.text("添加下载", "Add download"))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.add(window, cx)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_sm()
                            .text_color(rgb(muted))
                            .child(self.text("保存至", "Save to"))
                            .child(
                                div().flex_1().child(
                                    Input::new(&self.directory_input)
                                        .id("destination")
                                        .aria_label(self.text("保存位置", "Destination"))
                                        .w_full(),
                                ),
                            ),
                    ),
            );
            if self.auth_expanded {
                content = content.child(div().rounded_xl().border_1().border_color(rgb(border)).bg(rgb(side)).p_4().flex().flex_col().gap_3()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(self.text("FTP / SFTP 认证", "FTP / SFTP authentication")))
                    .child(div().flex().gap_3()
                        .child(self.settings_field(self.text("用户名", "Username"), "auth-user", &self.auth_user_input, muted))
                        .child(self.settings_field(self.text("密码", "Password"), "auth-password", &self.auth_password_input, muted)))
                    .child(div().flex().gap_3()
                        .child(self.settings_field(self.text("SFTP 私钥路径（可选）", "SFTP private key path (optional)"), "auth-key", &self.auth_key_input, muted))
                        .child(self.settings_field(self.text("私钥口令（可选）", "Key passphrase (optional)"), "auth-passphrase", &self.auth_passphrase_input, muted)))
                    .child(Button::new("ftp-security").label(match self.ftp_security {
                        FtpSecurity::Auto | FtpSecurity::Plain => self.text("FTP 安全：自动", "FTP security: Auto"),
                        FtpSecurity::ExplicitTls => self.text("FTP 安全：显式 TLS", "FTP security: Explicit TLS"),
                        FtpSecurity::ImplicitTls => self.text("FTP 安全：隐式 TLS", "FTP security: Implicit TLS"),
                    }).on_click(cx.listener(|this, _, _, cx| { this.ftp_security = match this.ftp_security { FtpSecurity::Auto | FtpSecurity::Plain => FtpSecurity::ExplicitTls, FtpSecurity::ExplicitTls => FtpSecurity::ImplicitTls, FtpSecurity::ImplicitTls => FtpSecurity::Auto }; cx.notify(); })))
                    .child(div().text_xs().text_color(rgb(muted)).child(self.text("密码和私钥口令会加密保存在本机。使用私钥时无需填写密码。", "Passwords and key passphrases are encrypted on this device. Leave password empty when using a private key."))));
            }
            if let Some((url, catalog)) = &self.hls_catalog
                && self.url_input.read(cx).value().as_ref().trim() == url
            {
                content = content.child(self.hls_options_panel(catalog, dark, cx));
            }
            if self.editing_url.is_some() {
                content = content.child(
                    div().rounded_xl().border_1().border_color(rgb(border)).bg(rgb(side))
                        .p_4().flex().flex_col().gap_3()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(self.text("更换下载链接", "Change download link")))
                        .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                            "请粘贴同一文件的新链接；跨域时会清除原有网站凭据。",
                            "Paste a new link to the same file; credentials are cleared when the origin changes.",
                        )))
                        .child(div().flex().gap_3().items_center()
                            .child(div().flex_1().child(Input::new(&self.replacement_input)
                                .id("replacement-url").aria_label(self.text("新下载链接", "New download link")).w_full()))
                            .child(Button::new("save-replacement-url").primary().label(self.text("保存", "Save"))
                                .on_click(cx.listener(|this, _, window, cx| this.save_replacement_url(window, cx))))
                            .child(Button::new("cancel-replacement-url").label(self.text("取消", "Cancel"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.editing_url = None;
                                    this.replacement_input.update(cx, |input, cx| input.set_value("", window, cx));
                                    cx.notify();
                                })))),
                );
            }
            if let Some(id) = self.deleting_task {
                let name = self
                    .tasks
                    .iter()
                    .find(|task| task.id == id)
                    .and_then(|task| task.request.suggested_filename.clone())
                    .unwrap_or_else(|| self.text("未命名下载", "Untitled download").into());
                content = content.child(
                    div()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(border))
                        .bg(rgb(side))
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(self.text("移除下载任务", "Remove download")),
                        )
                        .child(div().text_xs().text_color(rgb(muted)).child(format!(
                            "{} {name}",
                            self.text(
                                "是否同时删除此任务的临时文件？",
                                "Also delete temporary files for:"
                            )
                        )))
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .items_center()
                                .child(
                                    Button::new("remove-keep-partial")
                                        .label(self.text("保留临时文件", "Keep temporary files"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.delete_record(id, false, cx)
                                        })),
                                )
                                .child(
                                    Button::new("remove-delete-partial")
                                        .label(self.text("删除临时文件", "Delete temporary files"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.delete_record(id, true, cx)
                                        })),
                                )
                                .child(
                                    Button::new("cancel-remove")
                                        .label(self.text("取消", "Cancel"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.deleting_task = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                );
            }
            let mut list = div()
                .id("task-list")
                .flex_1()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3();
            if visible.is_empty() {
                list = list.child(
                    div()
                        .flex_1()
                        .min_h(px(160.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(rgb(muted))
                        .child(self.text("这里还没有下载任务", "No downloads here yet")),
                );
            } else {
                for task in &visible {
                    list = list.child(self.task_row(task, dark, cx));
                }
            }
            content = content.child(list);
        }
        if let Some(notice) = &self.notice {
            content = content.child(div().text_sm().text_color(rgb(muted)).child(notice.clone()));
        }
        div()
            .size_full()
            .flex()
            .bg(rgb(page))
            .text_color(rgb(fg))
            .child(
                div()
                    .w(px(220.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_5()
                    .bg(rgb(side))
                    .border_r_1()
                    .border_color(rgb(border))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .items_center()
                            .mb_8()
                            .child(
                                div()
                                    .size(px(40.))
                                    .rounded_lg()
                                    .bg(rgb(0x7465ef))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(rgb(0xffffff))
                                    .font_weight(FontWeight::BOLD)
                                    .child("N"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().font_weight(FontWeight::BOLD).child("NDM Next"))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(muted))
                                            .child(self.text("下载管理器", "Download manager")),
                                    ),
                            ),
                    )
                    .child(self.nav_item(self.text("全部任务", "All downloads"), Section::All, cx))
                    .child(self.nav_item(self.text("进行中", "In progress"), Section::Active, cx))
                    .child(self.nav_item(self.text("已完成", "Completed"), Section::Completed, cx))
                    .child(div().flex_1())
                    .child(self.nav_item(self.text("设置", "Settings"), Section::Settings, cx)),
            )
            .child(content)
    }
}
