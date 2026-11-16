use crate::{
    apply_theme,
    library::{Category, SortOrder, TaskIndex},
    preferences::{Language, Preferences, Theme},
    progress::ProgressWindow,
    protocol_auth::{AuthFields, apply as apply_protocol_auth},
    speed::SpeedMeter,
    updates::{self, InstallWait, UpdateState},
};
use gpui_kit::{
    component::{
        Icon, IconName, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputState},
        menu::{DropdownMenu, PopupMenuItem},
    },
    prelude::*,
    *,
};
use ndm_core::{
    DashCatalog, DashOptions, FtpSecurity, HlsCatalog, HlsOptions, NewTask, ProxyOptions,
    Scheduler, SchedulerEvent, Store, Task, TaskId, TaskOptions, TaskState,
    inspect_dash_with_options, inspect_hls_with_options,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

mod media;
mod settings;
mod storage;
mod tasks;
mod updating;

gpui_kit::actions!(ndm_next, [FocusSearch]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    All,
    Active,
    Completed,
    Settings,
}

enum MediaCatalog {
    Hls(HlsCatalog),
    Dash(DashCatalog),
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
    search_input: Entity<InputState>,
    search_query: String,
    task_index: TaskIndex,
    matching_tasks: Vec<usize>,
    category: Category,
    sort_order: SortOrder,
    settings_open: HashSet<usize>,
    replacement_input: Entity<InputState>,
    directory_input: Entity<InputState>,
    storage_download_input: Entity<InputState>,
    cache_input: Entity<InputState>,
    retention_input: Entity<InputState>,
    storage_busy: bool,
    cache_report: Option<ndm_core::CacheReport>,
    update_cache_gate: Arc<tokio::sync::Mutex<()>>,
    update_cache_lease: Option<tokio::sync::OwnedMutexGuard<()>>,
    active_input: Entity<InputState>,
    connections_input: Entity<InputState>,
    #[cfg(target_os = "macos")]
    extension_id_input: Entity<InputState>,
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
    pending_proxy_enabled: bool,
    pending_host_keys: HashMap<TaskId, PendingHostKey>,
    confirming_host_key: Option<TaskId>,
    editing_url: Option<TaskId>,
    delete_dialog: Option<AnyWindowHandle>,
    deleting: bool,
    selected_tasks: HashSet<TaskId>,
    sidebar_collapsed: bool,
    new_download_open: bool,
    hls_catalog: Option<(String, HlsCatalog)>,
    hls_variant: Option<usize>,
    hls_audio: Vec<String>,
    dash_catalog: Option<(String, DashCatalog)>,
    dash_variant: Option<usize>,
    dash_audio: Vec<usize>,
    inspecting_hls: bool,
    #[cfg(any(windows, target_os = "macos"))]
    tray: Option<crate::tray::Tray>,
    #[cfg(any(windows, target_os = "macos"))]
    startup_enabled: bool,
    section: Section,
    tasks: Vec<Task>,
    seen_tasks: HashSet<TaskId>,
    progress_windows: HashMap<TaskId, (AnyWindowHandle, WeakEntity<ProgressWindow>)>,
    retained_progress: HashMap<TaskId, ndm_core::ProgressSnapshot>,
    sample: HashMap<TaskId, (ndm_core::ProgressPhase, SpeedMeter)>,
    speed: HashMap<TaskId, u64>,
    tracking: bool,
    notice: crate::notice::Notice,
    notice_timer: Option<gpui_kit::Task<()>>,
    update_state: UpdateState,
    install_update_when_idle: bool,
    install_wait: Option<InstallWait>,
}

pub(crate) struct DesktopServices {
    pub store: Arc<Store>,
    pub scheduler: Scheduler,
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub preferences: Preferences,
    pub preferences_path: PathBuf,
    pub prepared_at_launch: HashSet<TaskId>,
    #[cfg(any(windows, target_os = "macos"))]
    pub ui_sender: tokio::sync::mpsc::UnboundedSender<crate::tray::Command>,
    #[cfg(any(windows, target_os = "macos"))]
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
            prepared_at_launch,
            #[cfg(any(windows, target_os = "macos"))]
            ui_sender,
            #[cfg(any(windows, target_os = "macos"))]
            mut commands,
        } = services;
        cx.bind_keys([KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-k"
            } else {
                "ctrl-k"
            },
            FocusSearch,
            Some("NDMNext"),
        )]);
        let url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("搜索 / Search"));
        cx.observe(&search_input, |this, input, cx| {
            let query = input.read(cx).value().to_string();
            if this.search_query != query {
                this.search_query = query;
                this.refilter();
                cx.notify();
            }
        })
        .detach();
        let replacement_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        let directory = preferences.download_dir.to_string_lossy().into_owned();
        let directory_input = cx.new(|cx| InputState::new(window, cx).default_value(directory));
        let storage_download_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preferences.download_dir.to_string_lossy().into_owned())
        });
        let cache_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preferences.cache_dir.to_string_lossy().into_owned())
        });
        let retention_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences.cache_retention_days.to_string())
        });
        let active_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences.max_active_tasks.to_string())
        });
        let connections_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences.max_connections.to_string())
        });
        #[cfg(target_os = "macos")]
        let extension_id_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("chrome://extensions → ID"));
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
        let pending_proxy_enabled = preferences.proxy_enabled_by_default;
        let sidebar_collapsed = preferences.sidebar_collapsed;
        #[cfg(any(windows, target_os = "macos"))]
        let startup_enabled = crate::startup::enabled().unwrap_or(false);
        #[cfg(any(windows, target_os = "macos"))]
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
        #[cfg(any(windows, target_os = "macos"))]
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
                        crate::tray::Command::Refresh => {
                            if view
                                .update(cx, |this, cx| this.discover_new_tasks(cx))
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
        let mut events = scheduler.subscribe();
        let tasks = store.list().unwrap_or_default();
        let seen_tasks = tasks
            .iter()
            .filter(|task| {
                task.state != TaskState::Prepared && !prepared_at_launch.contains(&task.id)
            })
            .map(|task| task.id)
            .collect();
        let pending_host_keys = store
            .pending_sftp_hosts()
            .unwrap_or_default()
            .into_iter()
            .map(|key| {
                (
                    key.id,
                    PendingHostKey {
                        observed: key.observed,
                        expected: key.expected,
                    },
                )
            })
            .collect();
        cx.spawn(async move |view, cx| {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if view.update(cx, |this, cx| {
                            this.reload();
                            match event {
                                SchedulerEvent::Started(id) => {
                                    this.retained_progress.remove(&id);
                                    this.sample.remove(&id);
                                    this.speed.remove(&id);
                                    this.track_progress(cx);
                                    if this.seen_tasks.insert(id) { this.show_progress(id, cx); }
                                }
                                SchedulerEvent::SftpHostKeyRequired { .. } => {
                                    this.set_notice(this.text("SFTP 服务器密钥需要核对。", "Check the SFTP server key before trusting it.").into(), cx);
                                },
                                SchedulerEvent::SftpHostKeyChanged { .. } => {
                                    this.set_notice(this.text("SFTP 服务器密钥已改变，请谨慎核对。", "The SFTP server key has changed; verify it carefully.").into(), cx);
                                },
                                SchedulerEvent::Finished { id, progress } => {
                                    this.retained_progress.insert(id, progress);
                                    this.speed.remove(&id);
                                    this.sample.remove(&id);
                                    this.refilter();
                                    this.try_install_update(cx);
                                }
                            }
                            cx.notify();
                        }).is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }).detach();
        let mut task_index = TaskIndex::default();
        task_index.rebuild(&tasks);
        let matching_tasks = task_index.filter("", Category::All, SortOrder::Recent);
        let mut this = Self {
            store,
            scheduler,
            runtime,
            preferences,
            preferences_path,
            url_input,
            search_input,
            search_query: String::new(),
            task_index,
            matching_tasks,
            category: Category::All,
            sort_order: SortOrder::Recent,
            settings_open: HashSet::new(),
            replacement_input,
            directory_input,
            storage_download_input,
            cache_input,
            retention_input,
            storage_busy: false,
            cache_report: None,
            update_cache_gate: Arc::new(tokio::sync::Mutex::new(())),
            update_cache_lease: None,
            active_input,
            connections_input,
            #[cfg(target_os = "macos")]
            extension_id_input,
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
            pending_proxy_enabled,
            pending_host_keys,
            confirming_host_key: None,
            editing_url: None,
            delete_dialog: None,
            deleting: false,
            selected_tasks: HashSet::new(),
            sidebar_collapsed,
            new_download_open: false,
            hls_catalog: None,
            hls_variant: None,
            hls_audio: Vec::new(),
            dash_catalog: None,
            dash_variant: None,
            dash_audio: Vec::new(),
            inspecting_hls: false,
            #[cfg(any(windows, target_os = "macos"))]
            tray,
            #[cfg(any(windows, target_os = "macos"))]
            startup_enabled,
            section: Section::All,
            tasks,
            seen_tasks,
            progress_windows: HashMap::new(),
            retained_progress: HashMap::new(),
            sample: HashMap::new(),
            speed: HashMap::new(),
            tracking: false,
            notice: crate::notice::Notice::default(),
            notice_timer: None,
            update_state: UpdateState::NotChecked,
            install_update_when_idle: false,
            install_wait: None,
        };
        if this
            .tasks
            .iter()
            .any(|task| task.state == TaskState::Running)
        {
            this.track_progress(cx);
        }
        this.start_update_watcher(cx);
        this.start_storage_watcher(cx);
        this
    }

    fn text(&self, chinese: &'static str, english: &'static str) -> &'static str {
        match self.preferences.language {
            Language::Chinese => chinese,
            Language::English => english,
        }
    }

    fn set_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.notice.show(message, Instant::now());
        // Replacing this handle cancels the previous timer, including identical messages.
        self.notice_timer = Some(cx.spawn(async move |view, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = view.update(cx, |this, cx| {
                if this.notice.expire(Instant::now()) {
                    cx.notify();
                }
            });
        }));
    }

    fn clear_notice(&mut self) {
        self.notice.clear();
        self.notice_timer = None;
    }

    pub(crate) fn delete_confirmation_closed(
        &mut self,
        handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.delete_dialog == Some(handle) {
            self.delete_dialog = None;
            self.try_install_update(cx);
            cx.notify();
        }
    }
    fn reload(&mut self) {
        if let Ok(tasks) = self.store.list() {
            self.tasks = tasks;
            self.task_index.update(&self.tasks);
            self.refilter();
            let task_ids: HashSet<_> = self
                .tasks
                .iter()
                .filter(|task| task.state != TaskState::Prepared)
                .map(|task| task.id)
                .collect();
            self.selected_tasks.retain(|id| task_ids.contains(id));
            self.retained_progress.retain(|id, _| task_ids.contains(id));
        }
        if let Ok(pending) = self.store.pending_sftp_hosts() {
            self.pending_host_keys = pending
                .into_iter()
                .map(|key| {
                    (
                        key.id,
                        PendingHostKey {
                            observed: key.observed,
                            expected: key.expected,
                        },
                    )
                })
                .collect();
        }
    }
    fn refilter(&mut self) {
        let mut totals = HashMap::new();
        if self.sort_order == SortOrder::Size {
            totals.extend(
                self.retained_progress
                    .iter()
                    .filter_map(|(&id, progress)| progress.total.map(|size| (id, size))),
            );
            for task in self
                .tasks
                .iter()
                .filter(|task| task.state == TaskState::Running)
            {
                if let Some(total) = self
                    .scheduler
                    .progress(task.id)
                    .and_then(|progress| progress.total)
                {
                    totals.insert(task.id, total);
                }
            }
        }
        self.matching_tasks = self.task_index.filter_with_totals(
            &self.search_query,
            self.category,
            self.sort_order,
            &totals,
        );
    }
    fn discover_new_tasks(&mut self, cx: &mut Context<Self>) {
        self.reload();
        let new_tasks: Vec<_> = self
            .tasks
            .iter()
            .filter(|task| task.state != TaskState::Prepared && !self.seen_tasks.contains(&task.id))
            .map(|task| task.id)
            .collect();
        for id in new_tasks {
            self.seen_tasks.insert(id);
            self.show_progress(id, cx);
        }
        cx.notify();
    }
    fn show_progress(&mut self, id: TaskId, cx: &mut Context<Self>) {
        if let Some((handle, _)) = self.progress_windows.get(&id)
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let store = self.store.clone();
        let scheduler = self.scheduler.clone();
        let runtime = self.runtime.clone();
        let preferences = self.preferences.clone();
        let bounds = Bounds::centered(
            None,
            size(
                px(crate::progress::WINDOW_WIDTH),
                px(crate::progress::COMPACT_HEIGHT),
            ),
            cx,
        );
        match gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(460.), px(136.))),
                ..gpui_kit::component::TitleBar::window_options()
            },
            cx,
            move |window, cx| {
                crate::chrome::round_window(window);
                cx.new(|cx| {
                    ProgressWindow::new(id, store, scheduler, runtime, preferences, window, cx)
                })
            },
        ) {
            Ok((handle, view)) => {
                // Opening a window while the browser owns focus is insufficient
                // on Windows. Explicit activation also restores minimized windows.
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                self.progress_windows.insert(id, (handle, view.downgrade()));
            }
            Err(_) => self.set_notice(
                self.text("无法打开进度窗口。", "Could not open the progress window.")
                    .into(),
                cx,
            ),
        }
    }
    fn track_progress(&mut self, cx: &mut Context<Self>) {
        if self.tracking {
            return;
        }
        self.tracking = true;
        cx.spawn(async move |view, cx| {
            let mut refresh_tick = 0_u8;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                refresh_tick = (refresh_tick + 1) % 10;
                let keep_tracking = view
                    .update(cx, |this, cx| {
                        if refresh_tick == 0 {
                            this.reload();
                        }
                        let mut any = false;
                        for task in &this.tasks {
                            if let Some(progress) = this.scheduler.progress(task.id) {
                                any = true;
                                let (phase, meter) = this
                                    .sample
                                    .entry(task.id)
                                    .or_insert_with(|| (progress.phase, SpeedMeter::default()));
                                if *phase != progress.phase {
                                    *phase = progress.phase;
                                    meter.reset();
                                }
                                let bytes = match progress.phase {
                                    ndm_core::ProgressPhase::Downloading => progress.transferred,
                                    ndm_core::ProgressPhase::Merging => progress.merged,
                                    ndm_core::ProgressPhase::Saving => progress.saved,
                                    ndm_core::ProgressPhase::Finalizing => 0,
                                };
                                this.speed
                                    .insert(task.id, meter.sample(Instant::now(), bytes));
                            } else {
                                this.sample.remove(&task.id);
                                this.speed.remove(&task.id);
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
        if matches!(self.update_state, UpdateState::Installing) {
            return;
        }
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
                self.set_notice(
                    self.text("请输入有效的下载链接。", "Enter a valid download URL.")
                        .into(),
                    cx,
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
                self.set_notice(
                    self.text(
                        "请检查 FTP/SFTP 凭据与连接方式。",
                        "Check the FTP/SFTP credentials and connection mode.",
                    )
                    .into(),
                    cx,
                );
                cx.notify();
                return;
            }
        };
        if destination.trim().is_empty() {
            self.set_notice(
                self.text("请选择保存位置。", "Choose a destination folder.")
                    .into(),
                cx,
            );
            cx.notify();
            return;
        }
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
        } else if matches!(protocol_options, TaskOptions::Auto)
            && self
                .dash_catalog
                .as_ref()
                .is_some_and(|(catalog_url, _)| catalog_url == &url)
        {
            TaskOptions::Dash(DashOptions {
                representation_index: self.dash_variant,
                audio_indices: self.dash_audio.clone(),
                ..Default::default()
            })
        } else {
            protocol_options
        };
        let request = NewTask {
            url: parsed.to_string(),
            destination: PathBuf::from(destination),
            suggested_filename: crate::filenames::manual_suggestion(&parsed, &options),
            source_url: None,
            expected_size: None,
            headers: BTreeMap::new(),
        };
        match self
            .store
            .prepare_with_options(id, &request, &options)
            .and_then(|_| self.store.commit(id))
        {
            Ok(_) => {
                self.scheduler.notify();
                self.seen_tasks.insert(id);
                self.show_progress(id, cx);

                self.set_notice(self.text("下载已添加。", "Download added.").into(), cx);
                self.section = Section::All;
                self.new_download_open = false;
                self.url_input
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.hls_catalog = None;
                self.hls_variant = None;
                self.hls_audio.clear();
                self.dash_catalog = None;
                self.dash_variant = None;
                self.dash_audio.clear();
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
            Err(_) => self.set_notice(
                self.text("无法添加下载任务。", "Could not add the download.")
                    .into(),
                cx,
            ),
        }
        cx.notify();
    }
    fn save_preferences(&mut self, cx: &mut Context<Self>) {
        if self.preferences.save(&self.preferences_path).is_err() {
            self.set_notice(
                self.text("无法保存设置。", "Could not save settings.")
                    .into(),
                cx,
            );
        }
    }
    fn set_theme(&mut self, mode: Theme, cx: &mut Context<Self>) {
        self.preferences.theme = mode;
        apply_theme(mode, cx);
        self.save_preferences(cx);
        cx.notify();
    }
    fn save_download_settings(&mut self, cx: &mut Context<Self>) {
        let active = self.active_input.read(cx).value().parse::<usize>();
        let connections = self.connections_input.read(cx).value().parse::<usize>();
        let speed = self.speed_input.read(cx).value().parse::<u64>();
        let (Ok(active), Ok(connections), Ok(speed)) = (active, connections, speed) else {
            self.set_notice(
                self.text("请输入有效的数字设置。", "Enter valid numeric settings.")
                    .into(),
                cx,
            );
            cx.notify();
            return;
        };
        if !(1..=32).contains(&active) || !(1..=16).contains(&connections) || speed > 1_000_000 {
            self.set_notice(
                self.text(
                    "任务数须为 1–32，连接数须为 1–16，限速须为 0–1000000。",
                    "Tasks must be 1–32, connections 1–16, and speed 0–1000000.",
                )
                .into(),
                cx,
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
            if !username.is_empty() || !password.is_empty() || self.pending_proxy_enabled {
                self.set_notice(
                    self.text("请先填写代理地址。", "Enter a proxy address first.")
                        .into(),
                    cx,
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
                self.set_notice(
                    self.text(
                        "代理地址须为 HTTP(S)，用户名和密码请单独填写。",
                        "Use an HTTP(S) proxy URL and enter credentials separately.",
                    )
                    .into(),
                    cx,
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
        updated.proxy_enabled_by_default = self.pending_proxy_enabled && updated.proxy.is_some();
        if updated.save(&self.preferences_path).is_err() {
            self.set_notice(
                self.text("无法保存设置。", "Could not save settings.")
                    .into(),
                cx,
            );
        } else {
            self.scheduler.update_options(updated.scheduler_options());
            self.preferences = updated;
            let proxy = self.preferences.proxy.clone();
            for (_, view) in self.progress_windows.values() {
                let _ = view.update(cx, |view, cx| {
                    view.configured_proxy = proxy.clone();
                    cx.notify();
                });
            }
            self.set_notice(
                self.text(
                    "设置已保存；正在运行的任务会保持原设置。",
                    "Settings saved; running downloads keep their current settings.",
                )
                .into(),
                cx,
            );
        }
        cx.notify();
    }
    #[cfg(any(windows, target_os = "macos"))]
    fn toggle_startup(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.startup_enabled;
        match crate::startup::set(enabled) {
            Ok(()) => {
                self.startup_enabled = enabled;
                self.preferences.start_at_login = Some(enabled);
                self.save_preferences(cx);
                self.set_notice(
                    self.text("开机自启设置已更新。", "Startup setting updated.")
                        .into(),
                    cx,
                );
            }
            Err(_) => self.set_notice(
                self.text(
                    "无法更新开机自启，请检查系统设置或其他安装。",
                    "Could not update startup; check system settings or another installation.",
                )
                .into(),
                cx,
            ),
        }
        cx.notify();
    }
    #[cfg(target_os = "macos")]
    fn repair_browser_integration(&mut self, cx: &mut Context<Self>) {
        let id = self.extension_id_input.read(cx).value().trim().to_owned();
        if id.len() != 32 || !id.bytes().all(|byte| (b'a'..=b'p').contains(&byte)) {
            self.set_notice(
                self.text(
                    "请输入扩展管理页显示的 32 位扩展 ID。",
                    "Enter the 32-character ID from your browser's extensions page.",
                )
                .into(),
                cx,
            );
            return;
        }
        let result = std::env::current_exe().and_then(|path| {
            ndm_host::macos::register_browser_hosts(&path.with_file_name("ndm-host"), &[id])
        });
        let message = if result.is_ok() {
            self.text(
                "已授权此扩展。请在扩展中选择 NDM Next，然后重试下载。",
                "Extension allowed. Select NDM Next in the extension and retry your download.",
            )
        } else {
            self.text(
                "连接修复失败，请检查应用安装位置和浏览器目录权限。",
                "Repair failed. Check the app location and browser directory permissions.",
            )
        };
        self.set_notice(message.into(), cx);
    }
    fn task_action(&mut self, id: TaskId, state: TaskState) {
        if matches!(self.update_state, UpdateState::Installing) {
            return;
        }
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
        match self.store.trust_sftp_host(id, &key.observed) {
            Ok(()) => {
                self.pending_host_keys.remove(&id);
                self.confirming_host_key = None;
                self.task_action(id, TaskState::Failed);
                self.set_notice(
                    self.text(
                        "服务器密钥已保存，正在重新连接。",
                        "Server key saved; reconnecting.",
                    )
                    .into(),
                    cx,
                );
            }
            Err(_) => {
                self.reload();
                self.confirming_host_key = None;
                self.set_notice(
                    self.text(
                        "无法保存服务器密钥，请重新核对。",
                        "Could not save the server key; review it again.",
                    )
                    .into(),
                    cx,
                )
            }
        }
        cx.notify();
    }
    fn confirm_delete(&mut self, ids: Vec<TaskId>, cx: &mut Context<Self>) {
        if matches!(self.update_state, UpdateState::Installing) || self.deleting || ids.is_empty() {
            return;
        }
        if let Some(handle) = self.delete_dialog
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| {
                self.tasks
                    .iter()
                    .any(|task| task.id == *id && task.state != TaskState::Prepared)
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let name = (ids.len() == 1)
            .then(|| {
                self.tasks
                    .iter()
                    .find(|task| task.id == ids[0])
                    .and_then(crate::filenames::display_name)
            })
            .flatten();
        match crate::confirm_delete::open(
            ids,
            name,
            self.preferences.language,
            cx.entity().downgrade(),
            cx,
        ) {
            Ok(handle) => self.delete_dialog = Some(handle),
            Err(()) => self.set_notice(
                self.text(
                    "无法打开删除确认窗口。",
                    "Could not open the delete confirmation.",
                )
                .into(),
                cx,
            ),
        }
        cx.notify();
    }

    pub(crate) fn delete_tasks(
        &mut self,
        ids: Vec<TaskId>,
        delete_files: bool,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.update_state, UpdateState::Installing) || self.deleting || ids.is_empty() {
            return;
        }
        self.deleting = true;
        self.set_notice(self.text("正在删除任务…", "Deleting downloads…").into(), cx);
        let store = self.store.clone();
        let scheduler = self.scheduler.clone();
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let result = runtime.spawn(crate::deletion::delete_many(store, scheduler, ids, delete_files)).await;
            let _ = view.update(cx, |this, cx| {
                this.deleting = false;
                match result {
                    Ok(report) => {
                        for id in &report.removed {
                            this.selected_tasks.remove(id);
                            this.pending_host_keys.remove(id);
                            if let Some((handle, _)) = this.progress_windows.remove(id) {
                                let _ = handle.update(cx, |_, window, _| window.remove_window());
                            }
                            if this.editing_url == Some(*id) { this.editing_url = None; }
                        }
                        this.set_notice(if report.failed.is_empty() {
                            format!("{} {} {}", this.text("已删除", "Deleted"), report.removed.len(), this.text("个任务。", "downloads."))
                        } else {
                            format!("{} {} · {} {}. {}", this.text("已删除", "Deleted"), report.removed.len(),
                                this.text("未完成", "Failed"), report.failed.len(),
                                this.text("未删除的记录已保留；部分文件可能已删除。", "Unremoved records were kept; some files may already have been deleted."))
                        }, cx);
                    }
                    Err(_) => this.set_notice(this.text("删除失败，请检查文件和目录权限。", "Deletion failed; check file and folder permissions.").into(), cx),
                }
                this.reload();
                this.try_install_update(cx);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn open_task_path(&mut self, id: TaskId, file: bool, cx: &mut Context<Self>) {
        let result = self
            .tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| std::io::Error::other("task not found"))
            .and_then(|task| {
                if file {
                    crate::files::open_file(task)
                } else {
                    crate::files::open_folder(task)
                }
            });
        if result.is_err() {
            self.set_notice(
                if file {
                    self.text(
                        "无法打开文件，请检查文件是否存在。",
                        "Could not open the file. Check that it still exists.",
                    )
                } else {
                    self.text(
                        "无法打开下载文件夹。",
                        "Could not open the download folder.",
                    )
                }
                .into(),
                cx,
            );
            cx.notify();
        }
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
                self.set_notice(
                    self.text(
                        "链接已更新，可以继续下载。",
                        "Link updated; you can resume the download.",
                    )
                    .into(),
                    cx,
                );
            }
            Err(_) => {
                self.set_notice(
                    self.text(
                        "链接无效，或任务已不再暂停。",
                        "The link is invalid or the task is no longer paused.",
                    )
                    .into(),
                    cx,
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

pub(crate) fn format_bytes(bytes: u64) -> String {
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
            .gap_1()
            .child(div().text_xs().text_color(rgb(muted)).child(label))
            .child(Input::new(input).small().id(id).aria_label(label).w_full())
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
        let icon = match section {
            Section::All => IconName::LayoutDashboard,
            Section::Active => IconName::ArrowDown,
            Section::Completed => IconName::CircleCheck,
            Section::Settings => IconName::Settings2,
        };
        let dark = self.is_dark(cx);
        let row = div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .when(self.sidebar_collapsed, |row| row.justify_center())
            .child(Icon::new(icon).with_size(px(15.)))
            .when(!self.sidebar_collapsed, |row| row.child(label));
        let button = Button::new(("nav", section as usize))
            .small()
            .w_full()
            .h(px(30.))
            .ghost()
            .child(row)
            .tooltip(label)
            .accessibility_label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.section != section {
                    this.selected_tasks.clear();
                    this.new_download_open = false;
                    this.clear_notice();
                }
                this.section = section;
                cx.notify();
            }));
        if selected {
            button
                .bg(rgb(if dark { 0x302b50 } else { 0xede9ff }))
                .text_color(rgb(if dark { 0xd9d1ff } else { 0x523fa7 }))
        } else {
            button
        }
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
            .matching_tasks
            .iter()
            .copied()
            .filter(|index| {
                let task = &self.tasks[*index];
                task.state != TaskState::Prepared
                    && match self.section {
                        Section::All => true,
                        Section::Active => matches!(
                            task.state,
                            TaskState::Queued | TaskState::Running | TaskState::Paused
                        ),
                        Section::Completed => task.state == TaskState::Completed,
                        Section::Settings => false,
                    }
            })
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
            .gap_2()
            .px_4()
            .py_3();
        let zh = self.preferences.language == Language::Chinese;
        let current_category = self.category;
        let current_sort = self.sort_order;
        let category_view = cx.entity().downgrade();
        let sort_view = cx.entity().downgrade();
        content = content.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(fg))
                                .child(title),
                        )
                        .child(div().text_xs().text_color(rgb(muted)).child(format!(
                            "{} {}",
                            count_active,
                            self.text("个任务正在运行", "active")
                        ))),
                )
                .when(self.section != Section::Settings, |row| {
                    row.child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search_input)
                                .small()
                                .id("search-downloads")
                                .w_full()
                                .prefix(Icon::new(IconName::Search).small())
                                .aria_label(self.text(
                                    "搜索文件名、来源或链接",
                                    "Search filename, source or URL",
                                )),
                        ),
                    )
                    .child(
                        Button::new("category-filter")
                            .small()
                            .icon(Icon::new(gpui_kit::assets::IconName::ListFilter))
                            .label(current_category.label(zh))
                            .dropdown_caret(true)
                            .dropdown_menu(move |menu, _, _| {
                                Category::ALL.iter().fold(menu, |menu, category| {
                                    let category = *category;
                                    let view = category_view.clone();
                                    menu.item(
                                        PopupMenuItem::new(category.label(zh))
                                            .checked(category == current_category)
                                            .on_click(move |_, _, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.category = category;
                                                    this.refilter();
                                                    this.selected_tasks.clear();
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                })
                            }),
                    )
                    .child(
                        Button::new("sort-downloads")
                            .small()
                            .icon(IconName::ArrowDown)
                            .tooltip(self.text("排序", "Sort"))
                            .accessibility_label(self.text("排序", "Sort"))
                            .dropdown_menu(move |menu, _, _| {
                                SortOrder::ALL.iter().fold(menu, |menu, sort| {
                                    let sort = *sort;
                                    let view = sort_view.clone();
                                    menu.item(
                                        PopupMenuItem::new(sort.label(zh))
                                            .checked(sort == current_sort)
                                            .on_click(move |_, _, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.sort_order = sort;
                                                    this.refilter();
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                })
                            }),
                    )
                    .child(
                        Button::new("new-download-toggle")
                            .small()
                            .primary()
                            .icon(if self.new_download_open {
                                IconName::Close
                            } else {
                                IconName::Plus
                            })
                            .tooltip(if self.new_download_open {
                                self.text("关闭新建下载", "Close new download")
                            } else {
                                self.text("新建下载", "New download")
                            })
                            .accessibility_label(self.text("新建下载", "New download"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if matches!(this.update_state, UpdateState::Installing) {
                                    return;
                                }
                                this.new_download_open = !this.new_download_open;
                                cx.notify();
                            })),
                    )
                }),
        );
        if self.section == Section::Settings {
            content = content
                .overflow_y_scroll()
                .child(self.settings_panel(dark, cx));
        } else {
            if let Some(id) = self.confirming_host_key
                && let Some(key) = self.pending_host_keys.get(&id)
            {
                content = content.child(
                    div()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(0x9b774a))
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_sm().child(self.text(
                            "请通过可信渠道核对 SFTP 服务器指纹",
                            "Verify the SFTP fingerprint through a trusted channel",
                        )))
                        .when_some(key.expected.clone(), |panel, expected| {
                            panel.child(div().text_xs().child(format!(
                                "{} {expected}",
                                self.text("原指纹：", "Previous:")
                            )))
                        })
                        .child(div().text_xs().child(format!(
                            "{} {}",
                            self.text("新指纹：", "Observed:"),
                            key.observed
                        )))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new("confirm-sftp-key")
                                        .small()
                                        .label(self.text("确认信任", "Confirm trust"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.trust_host_key(id, cx)
                                        })),
                                )
                                .child(
                                    Button::new("cancel-sftp-key")
                                        .small()
                                        .label(self.text("取消", "Cancel"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirming_host_key = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                );
            }
            if self.new_download_open || self.editing_url.is_some() {
                content = content.overflow_y_scroll();
            }
            if self.new_download_open {
                content = content.child(
                    div()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(border))
                        .bg(rgb(side))
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
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
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.inspect_media(cx)),
                                        ),
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
                if let Some((url, catalog)) = &self.dash_catalog
                    && self.url_input.read(cx).value().as_ref().trim() == url
                {
                    content = content.child(self.dash_options_panel(catalog, dark, cx));
                }
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
            let selectable: Vec<_> = visible
                .iter()
                .map(|index| &self.tasks[*index])
                .filter(|task| task.state != TaskState::Prepared)
                .map(|task| task.id)
                .collect();
            if !selectable.is_empty() {
                let all_selected = selectable.iter().all(|id| self.selected_tasks.contains(id));
                let selected_count = self.selected_tasks.len();
                let selected_ids: Vec<_> = self.selected_tasks.iter().copied().collect();
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("select-all")
                                .small()
                                .ghost()
                                .icon(if all_selected {
                                    IconName::CircleCheck
                                } else {
                                    IconName::Square
                                })
                                .label(if all_selected {
                                    self.text("取消全选", "Deselect all")
                                } else {
                                    self.text("全选", "Select all")
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    for id in &selectable {
                                        if all_selected {
                                            this.selected_tasks.remove(id);
                                        } else {
                                            this.selected_tasks.insert(*id);
                                        }
                                    }
                                    cx.notify();
                                })),
                        )
                        .when(selected_count > 0, |row| {
                            row.child(div().text_xs().text_color(rgb(muted)).child(format!(
                                "{} {}",
                                selected_count,
                                self.text("已选择", "selected")
                            )))
                            .child(
                                Button::new("delete-selected")
                                    .small()
                                    .danger()
                                    .label(self.text("删除", "Delete"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.confirm_delete(selected_ids.clone(), cx)
                                    })),
                            )
                        }),
                );
            }
            if visible.is_empty() {
                content = content.child(
                    div()
                        .flex_1()
                        .min_h(px(100.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(rgb(muted))
                        .child(
                            if self.search_query.is_empty() && self.category == Category::All {
                                self.text("这里还没有下载任务", "No downloads here yet")
                            } else {
                                self.text("没有符合条件的下载", "No matching downloads")
                            },
                        ),
                );
            } else {
                content = content.child(
                    uniform_list(
                        "task-list",
                        visible.len(),
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|index| {
                                    this.task_row(&this.tasks[visible[index]], dark, cx)
                                        .into_any_element()
                                })
                                .collect()
                        }),
                    )
                    .flex_1()
                    .min_h(px(if self.new_download_open { 120. } else { 0. })),
                );
            }
        }
        if let Some(notice) = self.notice.message() {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .bg(rgb(side))
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(muted))
                    .child(Icon::new(IconName::Info).with_size(px(13.)))
                    .child(div().flex_1().child(notice.to_owned()))
                    .child(
                        Button::new("dismiss-notice")
                            .small()
                            .ghost()
                            .icon(IconName::Close)
                            .accessibility_label(self.text("关闭提示", "Dismiss notification"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_notice();
                                cx.notify();
                            })),
                    ),
            );
        }
        div()
            .size_full()
            .key_context("NDMNext")
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                if this.section == Section::Settings {
                    this.section = Section::All;
                }
                this.search_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
            }))
            .flex()
            .flex_col()
            .bg(rgb(page))
            .text_color(rgb(fg))
            .child(crate::chrome::title_bar("NDM Next", dark).when(
                matches!(
                    self.update_state,
                    UpdateState::Available(_)
                        | UpdateState::Downloading
                        | UpdateState::Ready(_)
                        | UpdateState::Installing
                ) || self.remembered_update()
                    || (self.update_state == UpdateState::Failed && self.install_update_when_idle),
                |bar| {
                    bar.child(crate::chrome::title_bar_action(
                        self.title_update_button(cx),
                    ))
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .w(px(if self.sidebar_collapsed { 60. } else { 180. }))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_2()
                            .bg(rgb(side))
                            .border_r_1()
                            .border_color(rgb(border))
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .gap_2()
                                    .items_center()
                                    .h(px(36.))
                                    .mb_3()
                                    .when(self.sidebar_collapsed, |row| row.justify_center())
                                    .when(!self.sidebar_collapsed, |row| {
                                        row.child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .text_sm()
                                                .font_weight(FontWeight::BOLD)
                                                .child(crate::branding::logo(px(24.)))
                                                .child("NDM Next"),
                                        )
                                    })
                                    .when(!self.sidebar_collapsed, |row| row.child(div().flex_1()))
                                    .child(
                                        Button::new("sidebar-toggle")
                                            .small()
                                            .ghost()
                                            .w(px(28.))
                                            .h(px(28.))
                                            .flex_shrink_0()
                                            .icon(if self.sidebar_collapsed {
                                                IconName::PanelLeftOpen
                                            } else {
                                                IconName::PanelLeftClose
                                            })
                                            .tooltip(if self.sidebar_collapsed {
                                                self.text("展开侧边栏", "Expand sidebar")
                                            } else {
                                                self.text("收起侧边栏", "Collapse sidebar")
                                            })
                                            .accessibility_label(
                                                self.text("切换侧边栏", "Toggle sidebar"),
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.sidebar_collapsed = !this.sidebar_collapsed;
                                                this.preferences.sidebar_collapsed =
                                                    this.sidebar_collapsed;
                                                this.save_preferences(cx);
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(self.nav_item(
                                self.text("全部任务", "All downloads"),
                                Section::All,
                                cx,
                            ))
                            .child(self.nav_item(
                                self.text("进行中", "In progress"),
                                Section::Active,
                                cx,
                            ))
                            .child(self.nav_item(
                                self.text("已完成", "Completed"),
                                Section::Completed,
                                cx,
                            ))
                            .child(div().flex_1())
                            .child(self.nav_item(
                                self.text("设置", "Settings"),
                                Section::Settings,
                                cx,
                            )),
                    )
                    .child(content),
            )
    }
}
