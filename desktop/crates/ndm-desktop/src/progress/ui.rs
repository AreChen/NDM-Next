use super::*;
use gpui_kit::component::{
    IconName, Sizable, ThemeMode,
    button::{Button, ButtonVariants},
};

impl Render for ProgressWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = matches!(ComponentTheme::global(cx).mode, ThemeMode::Dark);
        let page = if dark { 0x101726 } else { 0xf8f9fd };
        let panel = if dark { 0x192337 } else { 0xffffff };
        let edge = if dark { 0x2c3a52 } else { 0xdfe6f2 };
        let fg = if dark { 0xf1f4ff } else { 0x19263c };
        let muted = if dark { 0x9baac4 } else { 0x71809a };
        let task = self.task.clone();
        let state = task
            .as_ref()
            .map(|task| task.state)
            .unwrap_or(TaskState::Failed);
        let active = matches!(
            state,
            TaskState::Running | TaskState::Queued | TaskState::Paused
        );
        let target_height = if self.expanded {
            EXPANDED_HEIGHT
        } else if active {
            COMPACT_HEIGHT
        } else {
            136.
        };
        let target_height = target_height + if self.notice.is_some() { 22. } else { 0. };
        if self.layout_height != target_height {
            self.layout_height = target_height;
            window.resize(size(px(WINDOW_WIDTH), px(target_height)));
        }
        let phase = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.phase)
            .unwrap_or_default();
        let accent = if state == TaskState::Completed {
            0x44cda6
        } else if state == TaskState::Failed {
            0xef8b9a
        } else {
            0x9c87ff
        };
        let name = task
            .as_ref()
            .and_then(crate::filenames::display_name)
            .unwrap_or_else(|| self.text("下载任务", "Download").into());
        let status = match (state, phase) {
            (TaskState::Prepared, _) => self.text("等待确认", "Awaiting confirmation"),
            (TaskState::Queued, _) => self.text("等待中", "Queued"),
            (TaskState::Running, ProgressPhase::Merging) => self.text("合并中", "Merging"),
            (TaskState::Running, ProgressPhase::Finalizing) => self.text("正在封装", "Finalizing"),
            (TaskState::Running, _) => self.text("下载中", "Downloading"),
            (TaskState::Paused, _) => self.text("已暂停", "Paused"),
            (TaskState::Completed, _)
                if task
                    .as_ref()
                    .is_some_and(|task| task.size_verified == Some(false)) =>
            {
                self.text("已完成 · 长度未校验", "Completed · length unverified")
            }
            (TaskState::Completed, _) => self.text("下载完成", "Completed"),
            (TaskState::Failed, _) => self.text("下载失败", "Failed"),
        };
        let downloaded = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.downloaded)
            .unwrap_or_else(|| {
                task.as_ref()
                    .and_then(|task| task.output_path.as_ref())
                    .and_then(|path| std::fs::metadata(path).ok())
                    .map(|metadata| metadata.len())
                    .unwrap_or(0)
            });
        let total = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.total)
            .or_else(|| task.as_ref().and_then(|task| task.request.expected_size));
        let shown_bytes = if state == TaskState::Running && phase == ProgressPhase::Merging {
            self.snapshot
                .as_ref()
                .map(|snapshot| snapshot.merged)
                .unwrap_or(0)
        } else {
            downloaded
        };
        let ratio = if state == TaskState::Completed {
            Some(1.)
        } else if phase == ProgressPhase::Finalizing {
            self.snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .finalization_total_ms
                    .filter(|total| *total > 0)
                    .map(|total| (snapshot.finalized_ms as f32 / total as f32).clamp(0., 0.999))
            })
        } else {
            total
                .filter(|total| *total > 0)
                .map(|total| (shown_bytes as f32 / total as f32).clamp(0., 0.999))
        };
        let percent = ratio
            .map(|value| format!("{:.1}%", value * 100.))
            .unwrap_or_else(|| self.text("处理中", "Working").into());
        let transferred = total
            .map(|total| format!("{} / {}", format_bytes(shown_bytes), format_bytes(total)))
            .unwrap_or_else(|| format_bytes(shown_bytes));
        let remaining = if state == TaskState::Running && phase == ProgressPhase::Finalizing {
            format_remaining(self.snapshot.as_ref().and_then(|snapshot| {
                media_remaining_seconds(
                    snapshot.finalization_total_ms,
                    snapshot.finalized_ms,
                    self.phase_started
                        .map(|start| start.elapsed().as_secs_f64())
                        .unwrap_or(0.),
                )
            }))
        } else if state == TaskState::Running {
            format_remaining(remaining_seconds(total, shown_bytes, self.speed))
        } else {
            "—".into()
        };
        let segments = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.segments.as_slice())
            .unwrap_or(&[]);
        let connections = segments.iter().filter(|segment| segment.active).count();
        let mut details = div().flex().flex_col().gap_2();
        if let Some(task) = &task {
            let host = reqwest::Url::parse(&task.request.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_default();
            for (label, value) in [
                (
                    self.text("保存位置", "Saved to"),
                    task.request.destination.to_string_lossy().into_owned(),
                ),
                (self.text("来源", "Source"), host),
                (self.text("状态", "Status"), status.to_owned()),
            ] {
                details = details.child(
                    div()
                        .flex()
                        .justify_between()
                        .gap_3()
                        .text_xs()
                        .child(div().text_color(rgb(muted)).child(label))
                        .child(div().max_w(px(300.)).truncate().child(value)),
                );
            }
            if reqwest::Url::parse(&task.request.url)
                .is_ok_and(|url| matches!(url.scheme(), "ftp" | "ftps" | "sftp"))
            {
                details = details.child(div().text_xs().text_color(rgb(muted)).child(self.text(
                    "继续下载时会核对已下载部分；无法可靠识别文件时需重新读取这些数据。",
                    "Resume verifies existing data; saved bytes may be reread when the server cannot prove file identity.")));
            }
            if let Some(error) = &task.error {
                details = details.child(
                    div().text_xs().text_color(rgb(0xee929d)).child(
                        crate::failure::description(error, self.language == Language::Chinese)
                            .to_owned(),
                    ),
                );
            }
        }
        details = details.child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(muted))
                .child(self.text("连接详情", "CONNECTIONS")),
        );
        if segments.is_empty() {
            details = details.child(
                div()
                    .text_xs()
                    .text_color(rgb(muted))
                    .child(self.text("此任务暂无分段信息", "No segment details for this task")),
            );
        }
        for (index, segment) in segments.iter().enumerate() {
            let bytes = segment
                .total
                .map(|total| {
                    format!(
                        "{} / {}",
                        format_bytes(segment.downloaded),
                        format_bytes(total)
                    )
                })
                .unwrap_or_else(|| format_bytes(segment.downloaded));
            let fraction = segment
                .total
                .filter(|total| *total > 0)
                .map(|total| (segment.downloaded as f32 / total as f32).clamp(0., 1.))
                .unwrap_or(0.);
            details = details.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(muted))
                    .child(div().w(px(30.)).child(format!("{:02}", index + 1)))
                    .child(
                        div()
                            .flex_1()
                            .h(px(3.))
                            .rounded_full()
                            .overflow_hidden()
                            .bg(rgb(edge))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(fraction))
                                    .bg(rgb(if segment.active { 0x44cda6 } else { accent })),
                            ),
                    )
                    .child(div().w(px(155.)).text_right().child(bytes)),
            );
        }
        let action = match state {
            TaskState::Running | TaskState::Queued => {
                Some((IconName::Pause, self.text("暂停", "Pause")))
            }
            TaskState::Paused | TaskState::Failed => {
                Some((IconName::Play, self.text("继续", "Resume")))
            }
            _ => None,
        };
        let fill = div()
            .h_full()
            .w(relative(ratio.unwrap_or(if state == TaskState::Running {
                1.
            } else {
                0.
            })))
            .rounded_full()
            .bg(linear_gradient(
                90.,
                linear_color_stop(rgb(0x885dff), 0.),
                linear_color_stop(
                    rgb(if state == TaskState::Completed {
                        0x3ed4b2
                    } else {
                        0x37c9f5
                    }),
                    1.,
                ),
            ));
        let fill = if ratio.is_none() && state == TaskState::Running {
            fill.with_animation(
                "unknown-size-progress",
                Animation::new(Duration::from_secs(2))
                    .repeat()
                    .with_easing(pulsating_between(0.15, 0.75)),
                |bar, opacity| bar.opacity(opacity),
            )
            .into_any_element()
        } else {
            fill.into_any_element()
        };
        let mut content = div()
            .flex_1()
            .min_h_0()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(name),
                            )
                            .child(
                                div().text_xs().text_color(rgb(muted)).truncate().child(
                                    div()
                                        .text_color(rgb(accent))
                                        .child(format!("{status}  ·  {transferred}")),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(accent))
                            .child(percent),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded_full()
                    .overflow_hidden()
                    .bg(rgb(edge))
                    .child(fill),
            );
        if active {
            let mut chips = div().flex().items_center().gap(px(6.));
            if phase == ProgressPhase::Finalizing {
                chips = chips.child(metric_chip(
                    self.text("封装", "Remux"),
                    self.text("本地处理", "Local processing").into(),
                    edge,
                    muted,
                ));
            } else {
                chips = chips.child(metric_chip(
                    if phase == ProgressPhase::Merging {
                        self.text("合并", "Merge")
                    } else {
                        self.text("速度", "Speed")
                    },
                    if state == TaskState::Running {
                        format!("{}/s", format_bytes(self.speed))
                    } else {
                        "—".into()
                    },
                    edge,
                    muted,
                ));
            }
            chips = chips
                .child(metric_chip(
                    self.text("剩余", "ETA"),
                    remaining,
                    edge,
                    muted,
                ))
                .child(metric_chip(
                    self.text("连接", "Peers"),
                    if segments.is_empty() {
                        "—".into()
                    } else {
                        format!("{connections}/{}", segments.len())
                    },
                    edge,
                    muted,
                ));
            let peak = self.history.iter().copied().max().unwrap_or(1).max(1);
            let mut graph = div().ml_auto().h(px(20.)).flex().items_end().gap(px(2.));
            for value in &self.history {
                graph = graph.child(
                    div()
                        .w(px(2.))
                        .h(px(2. + 16. * (*value as f32 / peak as f32)))
                        .rounded_full()
                        .bg(rgb(accent)),
                );
            }
            content = content.child(chips.child(graph));
        }
        content = content.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .when_some(action, |row, (icon, label)| {
                    row.child(
                        Button::new("progress-action")
                            .small()
                            .primary()
                            .icon(icon)
                            .label(label)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.change_state(state, cx)),
                            ),
                    )
                })
                .child(
                    Button::new("progress-folder")
                        .small()
                        .icon(IconName::FolderOpen)
                        .label(self.text("文件夹", "Folder"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_folder(cx))),
                )
                .when(state == TaskState::Completed, |row| {
                    row.child(
                        Button::new("progress-file")
                            .small()
                            .icon(IconName::ExternalLink)
                            .label(self.text("打开文件", "Open file"))
                            .on_click(cx.listener(|this, _, _, cx| this.open_file(cx))),
                    )
                })
                .child(div().flex_1())
                .child(
                    Button::new("progress-details")
                        .small()
                        .label(self.text("详情", "Details"))
                        .icon(if self.expanded {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.expanded = !this.expanded;
                            cx.notify();
                        })),
                ),
        );
        if let Some(notice) = &self.notice {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(0xe99561))
                    .truncate()
                    .child(notice.clone()),
            );
        }
        if self.expanded {
            content = content.child(
                div()
                    .id("progress-details-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(edge))
                    .bg(rgb(panel))
                    .p_2()
                    .child(details),
            );
        }
        div()
            .id("download-progress")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(page))
            .text_color(rgb(fg))
            .child(chrome::title_bar("NDM Next", dark))
            .child(content)
    }
}

fn metric_chip(label: &'static str, value: String, edge: u32, muted: u32) -> impl IntoElement {
    div()
        .h(px(24.))
        .px(px(7.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded_md()
        .border_1()
        .border_color(rgb(edge))
        .child(div().text_size(px(10.)).text_color(rgb(muted)).child(label))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
}
