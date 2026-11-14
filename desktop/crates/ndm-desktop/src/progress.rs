mod ui;

use crate::{chrome, files, preferences::Language, speed::SpeedMeter, view::format_bytes};
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
pub(crate) const EXPANDED_HEIGHT: f32 = 430.;

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

pub(crate) struct ProgressWindow {
    id: TaskId,
    store: Arc<Store>,
    scheduler: Scheduler,
    runtime: Arc<tokio::runtime::Runtime>,
    pub(crate) language: Language,
    task: Option<Task>,
    snapshot: Option<ProgressSnapshot>,
    meter: SpeedMeter,
    measured_phase: Option<ProgressPhase>,
    phase_started: Option<Instant>,
    history: VecDeque<u64>,
    speed: u64,
    expanded: bool,
    tracking: bool,
    notice: Option<String>,
    layout_height: f32,
}

impl ProgressWindow {
    pub(crate) fn new(
        id: TaskId,
        store: Arc<Store>,
        scheduler: Scheduler,
        runtime: Arc<tokio::runtime::Runtime>,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut events = scheduler.subscribe();
        let task = store.get(id).ok().flatten();
        let snapshot = scheduler.progress(id);
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
            language,
            task,
            snapshot,
            meter: SpeedMeter::default(),
            measured_phase: None,
            phase_started: None,
            history: VecDeque::new(),
            speed: 0,
            expanded: false,
            tracking: false,
            notice: None,
            layout_height: COMPACT_HEIGHT,
        };
        if this.snapshot.is_some() {
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
                            this.tracking = false;
                            return false;
                        };
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
    use super::{media_remaining_seconds, remaining_seconds};

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
