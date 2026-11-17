mod ui;

use crate::{
    chrome, files,
    preferences::{Language, Preferences},
    speed::SpeedMeter,
    view::format_bytes,
};
use gpui_kit::{component::Theme as ComponentTheme, prelude::*, *};
use ndm_core::{
    ProgressPhase, ProgressSnapshot, Scheduler, SchedulerEvent, Store, Task, TaskId, TaskState,
};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) const WINDOW_WIDTH: f32 = 480.;
pub(crate) const COMPACT_HEIGHT: f32 = 168.;
pub(crate) const FINISHED_HEIGHT: f32 = 136.;
pub(crate) const EXPANDED_HEIGHT: f32 = 430.;

fn should_track(state: Option<TaskState>, has_worker: bool) -> bool {
    has_worker || matches!(state, Some(TaskState::Queued | TaskState::Running))
}

fn remaining_seconds(total: Option<u64>, downloaded: u64, speed: u64) -> Option<u64> {
    total
        .filter(|_| speed > 0)
        .map(|total| total.saturating_sub(downloaded).div_ceil(speed))
}

fn media_remaining_seconds(total_ms: Option<u64>, processed_ms: u64, elapsed: f64) -> Option<u64> {
    total_ms
        .filter(|_| processed_ms > 0 && elapsed.is_finite() && elapsed > 0.)
        .map(|total| {
            ((total.saturating_sub(processed_ms) as f64 / processed_ms as f64) * elapsed).ceil()
                as u64
        })
}

fn format_remaining(seconds: Option<u64>) -> String {
    match seconds {
        Some(seconds) if seconds >= 3600 => {
            format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
        }
        Some(seconds) if seconds >= 60 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        Some(seconds) => format!("{seconds}s"),
        None => "—".into(),
    }
}

fn format_elapsed(milliseconds: Option<u64>) -> String {
    let Some(milliseconds) = milliseconds.filter(|value| *value > 0) else {
        return "—".into();
    };
    let seconds = milliseconds / 1_000;
    if seconds >= 3_600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3_600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

fn output_file_size(path: &std::path::Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
}

fn progress_bytes(
    task: Option<&Task>,
    snapshot: Option<&ProgressSnapshot>,
    completed_size: Option<u64>,
) -> (u64, Option<u64>) {
    if task.is_some_and(|task| task.state == TaskState::Completed)
        && let Some(size) = completed_size
    {
        return (size, Some(size));
    }
    (
        snapshot.map_or(0, |value| value.downloaded),
        snapshot
            .and_then(|value| value.total)
            .or_else(|| task.and_then(|task| task.request.expected_size)),
    )
}

pub(crate) struct ProgressWindow {
    id: TaskId,
    store: Arc<Store>,
    scheduler: Scheduler,
    runtime: Arc<tokio::runtime::Runtime>,
    pub(crate) language: Language,
    pub(crate) configured_proxy: Option<ndm_core::ProxyOptions>,
    proxy_switching: bool,
    task: Option<Task>,
    snapshot: Option<ProgressSnapshot>,
    completed_size: Option<u64>,
    meter: SpeedMeter,
    measured_phase: Option<ProgressPhase>,
    phase_started: Option<Instant>,
    history: VecDeque<u64>,
    speed: u64,
    expanded: bool,
    tracking: bool,
    notice: Option<String>,
    layout_height: f32,
    bar_motion: gpui_motion_kit::MotionState,
    bar_epoch: Instant,
}

impl ProgressWindow {
    pub(crate) fn new(
        id: TaskId,
        store: Arc<Store>,
        scheduler: Scheduler,
        runtime: Arc<tokio::runtime::Runtime>,
        preferences: Preferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut events = scheduler.subscribe();
        let task = store.get(id).ok().flatten();
        let snapshot = scheduler
            .progress(id)
            .or_else(|| task.as_ref().and_then(ndm_core::recover_task_progress));
        window.set_window_title("NDM Next · Download");
        cx.observe_global::<ComponentTheme>(|_, cx| cx.notify())
            .detach();
        cx.spawn(async move |view, cx| {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if view
                            .update(cx, |this, cx| match event {
                                SchedulerEvent::Started(task_id) if task_id == this.id => {
                                    this.task = this.store.get(this.id).ok().flatten();
                                    if let Some(task) = &mut this.task {
                                        task.state = TaskState::Running;
                                    }
                                    this.begin_tracking(cx);
                                    cx.notify();
                                }
                                SchedulerEvent::Finished {
                                    id: task_id,
                                    progress,
                                } if task_id == this.id => {
                                    this.task = this.store.get(this.id).ok().flatten();
                                    this.snapshot = Some(progress);
                                    this.cache_output_size(cx);
                                    this.speed = 0;
                                    this.meter.reset();
                                    this.measured_phase = None;
                                    this.phase_started = None;
                                    cx.notify();
                                }
                                _ => {}
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .detach();
        let mut this = Self {
            id,
            store,
            scheduler,
            runtime,
            language: preferences.language,
            configured_proxy: preferences.proxy,
            proxy_switching: false,
            task,
            snapshot,
            completed_size: None,
            meter: SpeedMeter::default(),
            measured_phase: None,
            phase_started: None,
            history: VecDeque::new(),
            speed: 0,
            expanded: false,
            tracking: false,
            notice: None,
            // First render fits the actual client area for this task's state.
            layout_height: 0.,
            bar_motion: Default::default(),
            bar_epoch: Instant::now(),
        };
        this.cache_output_size(cx);
        if should_track(
            this.task.as_ref().map(|task| task.state),
            this.scheduler.progress(id).is_some(),
        ) {
            this.begin_tracking(cx);
        }
        this
    }

    fn text(&self, chinese: &'static str, english: &'static str) -> &'static str {
        if self.language == Language::Chinese {
            chinese
        } else {
            english
        }
    }

    fn cache_output_size(&mut self, cx: &mut Context<Self>) {
        self.completed_size = None;
        let Some(path) = self
            .task
            .as_ref()
            .filter(|task| task.state == TaskState::Completed)
            .and_then(|task| task.output_path.clone())
        else {
            return;
        };
        let source = path.clone();
        cx.spawn(async move |view, cx| {
            let size = cx
                .background_executor()
                .spawn(async move { output_file_size(&source) })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.task.as_ref().is_some_and(|task| {
                    task.state == TaskState::Completed && task.output_path.as_ref() == Some(&path)
                }) {
                    this.completed_size = size;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn begin_tracking(&mut self, cx: &mut Context<Self>) {
        if self.tracking {
            return;
        }
        self.tracking = true;
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let keep = view
                    .update(cx, |this, cx| {
                        let Some(snapshot) = this.scheduler.progress(this.id) else {
                            this.task = this.store.get(this.id).ok().flatten();
                            this.tracking =
                                should_track(this.task.as_ref().map(|task| task.state), false);
                            if !this.tracking {
                                this.cache_output_size(cx);
                                this.speed = 0;
                                this.snapshot = this
                                    .task
                                    .as_ref()
                                    .and_then(ndm_core::recover_task_progress)
                                    .or_else(|| this.snapshot.clone());
                            }
                            cx.notify();
                            return this.tracking;
                        };
                        if let Some(task) = &mut this.task {
                            task.state = TaskState::Running;
                        }
                        if this.measured_phase != Some(snapshot.phase) {
                            this.measured_phase = Some(snapshot.phase);
                            this.phase_started = Some(Instant::now());
                            this.meter.reset();
                            this.history.clear();
                        }
                        this.speed = match snapshot.phase {
                            ProgressPhase::Downloading => {
                                this.meter.sample(Instant::now(), snapshot.transferred)
                            }
                            ProgressPhase::Merging => {
                                this.meter.sample(Instant::now(), snapshot.merged)
                            }
                            ProgressPhase::Finalizing => 0,
                            ProgressPhase::Saving => {
                                this.meter.sample(Instant::now(), snapshot.saved)
                            }
                        };
                        this.history.push_back(this.speed);
                        if this.history.len() > 24 {
                            this.history.pop_front();
                        }
                        this.snapshot = Some(snapshot);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    fn change_state(&mut self, state: TaskState, cx: &mut Context<Self>) {
        if self.proxy_switching {
            return;
        }
        let scheduler = self.scheduler.clone();
        let runtime = self.runtime.clone();
        let id = self.id;
        cx.spawn(async move |view, cx| {
            let result = runtime
                .spawn(async move {
                    if matches!(state, TaskState::Running | TaskState::Queued) {
                        scheduler.pause(id).await
                    } else {
                        scheduler.resume(id).await
                    }
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.task = this.store.get(id).ok().flatten();
                this.snapshot = this
                    .scheduler
                    .progress(id)
                    .or_else(|| this.task.as_ref().and_then(ndm_core::recover_task_progress));
                if !matches!(result, Ok(Ok(()))) {
                    this.notice = Some(
                        this.text("操作失败，请重试。", "Could not change the download state.")
                            .into(),
                    );
                }
                if this
                    .task
                    .as_ref()
                    .is_some_and(|task| task.state != TaskState::Running)
                {
                    this.speed = 0;
                    this.meter.reset();
                    this.measured_phase = None;
                }
                if should_track(
                    this.task.as_ref().map(|task| task.state),
                    this.scheduler.progress(id).is_some(),
                ) {
                    this.begin_tracking(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn enable_task_proxy(&mut self, cx: &mut Context<Self>) {
        if self.proxy_switching {
            return;
        }
        let Some(proxy) = self.configured_proxy.clone() else {
            self.notice = Some(
                self.text(
                    "请先在设置中配置下载代理。",
                    "Configure a download proxy in Settings first.",
                )
                .into(),
            );
            cx.notify();
            return;
        };
        self.proxy_switching = true;
        self.notice = None;
        let scheduler = self.scheduler.clone();
        let runtime = self.runtime.clone();
        let id = self.id;
        cx.spawn(async move |view, cx| {
            let result = runtime
                .spawn(async move { scheduler.retry_with_proxy(id, proxy).await })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.proxy_switching = false;
                this.task = this.store.get(id).ok().flatten();
                this.notice = if matches!(result, Ok(Ok(()))) {
                    None
                } else {
                    Some(
                        this.text(
                            "无法切换代理，请检查任务状态后重试。",
                            "Could not switch proxy. Check the download state and retry.",
                        )
                        .into(),
                    )
                };
                if should_track(
                    this.task.as_ref().map(|task| task.state),
                    this.scheduler.progress(id).is_some(),
                ) {
                    this.begin_tracking(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_folder(&mut self, cx: &mut Context<Self>) {
        if self
            .task
            .as_ref()
            .is_none_or(|task| files::open_folder(task).is_err())
        {
            self.notice = Some(
                self.text(
                    "无法打开下载文件夹。",
                    "Could not open the download folder.",
                )
                .into(),
            );
            cx.notify();
        }
    }

    fn open_file(&mut self, cx: &mut Context<Self>) {
        if self
            .task
            .as_ref()
            .is_none_or(|task| files::open_file(task).is_err())
        {
            self.notice = Some(
                self.text(
                    "无法打开文件，请检查文件是否存在。",
                    "Could not open the file. Check that it still exists.",
                )
                .into(),
            );
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_elapsed, media_remaining_seconds, progress_bytes, remaining_seconds, should_track,
    };
    use ndm_core::{NewTask, ProgressSnapshot, Task, TaskId, TaskOptions, TaskState};

    #[test]
    fn a_new_window_polls_queued_tasks_even_when_started_was_already_broadcast() {
        assert!(should_track(Some(TaskState::Queued), false));
        assert!(should_track(Some(TaskState::Queued), true));
        assert!(should_track(Some(TaskState::Running), true));
        assert!(should_track(Some(TaskState::Running), false));
        for state in [
            TaskState::Prepared,
            TaskState::Paused,
            TaskState::Completed,
            TaskState::Failed,
        ] {
            assert!(!should_track(Some(state), false));
        }
        assert!(!should_track(None, false));
    }

    #[test]
    fn paused_window_preserves_saved_bytes_and_known_total() {
        let snapshot = ProgressSnapshot {
            downloaded: 42,
            total: Some(100),
            ..Default::default()
        };
        assert_eq!(progress_bytes(None, Some(&snapshot), None), (42, Some(100)));
        assert_eq!(format_elapsed(Some(65_999)), "01:05");
        assert_eq!(format_elapsed(Some(3_661_000)), "1:01:01");
        assert_eq!(format_elapsed(None), "—");
    }

    #[test]
    fn completed_window_uses_actual_output_size_even_when_server_length_was_unknown() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("archive.zip");
        std::fs::write(&output, [0; 123]).unwrap();
        let task = Task {
            cache_directory: None,
            id: TaskId::new_v4(),
            request: NewTask {
                url: "https://example.test/archive.zip".into(),
                destination: directory.path().into(),
                suggested_filename: None,
                source_url: None,
                expected_size: None,
                headers: Default::default(),
            },
            options: TaskOptions::Auto,
            state: TaskState::Completed,
            output_path: Some(output),
            error: None,
            size_verified: Some(false),
            progress: None,
        };
        let old_snapshot = ProgressSnapshot {
            downloaded: 100,
            ..Default::default()
        };
        let measured_size = super::output_file_size(task.output_path.as_ref().unwrap());
        std::fs::remove_file(task.output_path.as_ref().unwrap()).unwrap();
        assert_eq!(
            progress_bytes(Some(&task), Some(&old_snapshot), measured_size),
            (123, Some(123))
        );
        assert_eq!(task.size_verified, Some(false));
        assert_eq!(old_snapshot.average_bytes_per_second(), None);
    }

    #[test]
    fn remaining_time_uses_measured_bytes_and_speed() {
        assert_eq!(remaining_seconds(Some(1_000), 400, 200), Some(3));
        assert_eq!(remaining_seconds(Some(1_000), 400, 0), None);
        assert_eq!(remaining_seconds(None, 400, 200), None);
        assert_eq!(remaining_seconds(Some(1_000), 1_100, 200), Some(0));
    }

    #[test]
    fn remux_remaining_time_uses_reported_timeline_and_actual_elapsed_time() {
        assert_eq!(media_remaining_seconds(Some(60_000), 20_000, 2.), Some(4));
        assert_eq!(media_remaining_seconds(Some(60_000), 20_000, 4.), Some(8));
        assert_eq!(media_remaining_seconds(None, 20_000, 2.), None);
        assert_eq!(media_remaining_seconds(Some(60_000), 0, 2.), None);
        assert_eq!(media_remaining_seconds(Some(60_000), 80_000, 2.), Some(0));
    }
}
