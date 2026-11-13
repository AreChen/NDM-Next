use crate::{
    apply_theme,
    preferences::{Language, Preferences, Theme},
};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        input::{Input, InputState},
    },
    prelude::*,
    *,
};
use ndm_core::{NewTask, Scheduler, SchedulerEvent, Store, Task, TaskId, TaskState};
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

pub(crate) struct Desktop {
    store: Arc<Store>,
    scheduler: Scheduler,
    runtime: Arc<tokio::runtime::Runtime>,
    preferences: Preferences,
    preferences_path: PathBuf,
    url_input: Entity<InputState>,
    directory_input: Entity<InputState>,
    section: Section,
    tasks: Vec<Task>,
    sample: HashMap<TaskId, (u64, Instant)>,
    speed: HashMap<TaskId, u64>,
    tracking: bool,
    notice: Option<String>,
}

impl Desktop {
    pub(crate) fn new(
        store: Arc<Store>,
        scheduler: Scheduler,
        runtime: Arc<tokio::runtime::Runtime>,
        preferences: Preferences,
        preferences_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        let directory = preferences.download_dir.to_string_lossy().into_owned();
        let directory_input = cx.new(|cx| InputState::new(window, cx).default_value(directory));
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
                                SchedulerEvent::SftpHostKeyRequired { fingerprint, .. } =>
                                    this.notice = Some(format!("SFTP host key needs confirmation: {fingerprint}")),
                                SchedulerEvent::SftpHostKeyChanged { expected, observed, .. } =>
                                    this.notice = Some(format!("SFTP host key changed. Expected {expected}; received {observed}")),
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
            directory_input,
            section: Section::All,
            tasks,
            sample: HashMap::new(),
            speed: HashMap::new(),
            tracking: false,
            notice: None,
        };
        if this
            .tasks
            .iter()
            .any(|task| task.state == TaskState::Running)
        {
            this.track_progress(cx);
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
    fn add(&mut self, cx: &mut Context<Self>) {
        let url = self
            .url_input
            .read(cx)
            .value()
            .to_string()
            .trim()
            .to_owned();
        let destination = self.directory_input.read(cx).value().to_string();
        let parsed = match reqwest::Url::parse(&url) {
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
            url,
            destination: PathBuf::from(destination),
            suggested_filename: filename,
            source_url: None,
            expected_size: None,
            headers: BTreeMap::new(),
        };
        let id = TaskId::new_v4();
        match self
            .store
            .prepare(id, &request)
            .and_then(|_| self.store.commit(id))
        {
            Ok(_) => {
                self.scheduler.notify();
                self.preferences.download_dir = request.destination.clone();
                self.save_preferences();
                self.notice = Some(self.text("下载已添加。", "Download added.").into());
                self.section = Section::All;
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
    fn delete_record(&mut self, id: TaskId, cx: &mut Context<Self>) {
        match self.store.delete(id) {
            Ok(()) => {
                self.reload();
                self.notice = Some(
                    self.text(
                        "记录已移除，文件已保留。",
                        "Record removed; files were kept.",
                    )
                    .into(),
                );
            }
            Err(_) => {
                self.notice = Some(self.text("请先暂停任务。", "Pause the task first.").into())
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
        let detail = match self.scheduler.progress(task.id) {
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
            None => task.error.clone().unwrap_or_else(|| status.into()),
        };
        let id = task.id;
        let state = task.state;
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
            .when(
                !matches!(state, TaskState::Running | TaskState::Prepared),
                |row| {
                    row.child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .label(self.text("移除", "Remove"))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.delete_record(id, cx)),
                            ),
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
        let mut content = div().flex_1().min_w_0().flex().flex_col().gap_5().p_8();
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
        if self.section == Section::Settings {
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
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            );
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
                                Button::new("add-download")
                                    .primary()
                                    .label(self.text("添加下载", "Add download"))
                                    .on_click(cx.listener(|this, _, _, cx| this.add(cx))),
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
