use super::*;
use gpui_kit::component::{
    Disableable, IconName, Sizable,
    button::ButtonVariants,
    menu::{DropdownMenu, PopupMenuItem},
};

impl Render for ProgressWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tone = crate::motion::ThemeTone::current(cx);
        let panel = tone.rgb(0x192337, 0xffffff);
        let edge = tone.rgb(0x2c3a52, 0xdfe6f2);
        let fg = tone.rgb(0xf1f4ff, 0x19263c);
        let muted = tone.rgb(0x9baac4, 0x71809a);
        let task = self.task.clone();
        let sharing = self.scheduler.sharing_status(self.id);
        let can_share = task
            .as_ref()
            .is_some_and(|task| self.scheduler.can_share(task));
        let chinese = self.language == Language::Chinese;
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
            FINISHED_HEIGHT
        };
        let target_height = target_height + if self.notice.is_some() { 22. } else { 0. };
        if self.layout_height != target_height {
            self.layout_height = target_height;
            chrome::fit_content(window, size(px(WINDOW_WIDTH), px(target_height)), cx);
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
            tone.rgb(0x9c87ff, 0x7152b4)
        };
        let name = task
            .as_ref()
            .and_then(crate::filenames::display_name)
            .unwrap_or_else(|| self.text("下载任务", "Download").into());
        let status = match (state, phase) {
            (TaskState::Prepared, _) => self.text("等待确认", "Awaiting confirmation"),
            (TaskState::Queued, _) => self.text("等待中", "Queued"),
            (TaskState::Running, ProgressPhase::Merging) => self.text("合并中", "Merging"),
            (TaskState::Running, ProgressPhase::Saving) => self.text("正在保存", "Saving"),
            (TaskState::Running, ProgressPhase::Finalizing) => self.text("正在封装", "Finalizing"),
            (TaskState::Running, _) => self.text("下载中", "Downloading"),
            (TaskState::Paused, _) => self.text("已暂停", "Paused"),
            (TaskState::Completed, _) => sharing
                .map_or(self.text("下载完成", "Completed"), |status| {
                    crate::peer_metrics::status(status, chinese)
                }),
            (TaskState::Failed, _) => self.text("下载失败", "Failed"),
        };
        let (downloaded, mut total) =
            progress_bytes(task.as_ref(), self.snapshot.as_ref(), self.completed_size);
        let shown_bytes = if state == TaskState::Running && phase == ProgressPhase::Saving {
            total = self.snapshot.as_ref().and_then(|value| value.save_total);
            self.snapshot.as_ref().map_or(0, |value| value.saved)
        } else if state == TaskState::Running && phase == ProgressPhase::Merging {
            self.snapshot
                .as_ref()
                .map(|snapshot| snapshot.merged)
                .unwrap_or(0)
        } else {
            downloaded
        };
        let ratio = if state == TaskState::Completed {
            Some(1.)
        } else if state == TaskState::Running && phase == ProgressPhase::Finalizing {
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
            .unwrap_or_else(|| {
                if state == TaskState::Running {
                    self.text("处理中", "Working").into()
                } else {
                    "—".into()
                }
            });
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
        let active_segments = segments.iter().filter(|segment| segment.active).count();
        let peers = self.snapshot.as_ref().and_then(|snapshot| snapshot.peers);
        let mut details = div().flex().flex_col().gap_2();
        if let Some(task) = &task {
            let host = reqwest::Url::parse(&task.request.url)
                .ok()
                .and_then(|url| match url.scheme() {
                    "magnet" => Some(self.text("磁力链接", "Magnet link").into()),
                    "file" => url.to_file_path().ok().and_then(|path| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    }),
                    _ => url.host_str().map(str::to_owned),
                })
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
        details = details.child(div().text_xs().text_color(rgb(muted)).child(self.text(
            "耗时不含暂停与排队。",
            "Elapsed excludes pauses and queue time.",
        )));
        details = details.child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(muted))
                .child(if peers.is_some() {
                    self.text("连接详情", "CONNECTIONS")
                } else {
                    self.text("分段详情", "SEGMENTS")
                }),
        );
        if let Some(peers) = peers {
            if self.expanded {
                let metrics = crate::peer_metrics::details(peers, downloaded, chinese);
                details = details.child(div().flex().flex_col().gap_2().children(
                    metrics.chunks(2).map(|pair| {
                        div()
                            .flex()
                            .gap_4()
                            .children(pair.iter().map(|(label, value)| {
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .justify_between()
                                    .gap_2()
                                    .text_xs()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_color(rgb(muted))
                                            .child(*label),
                                    )
                                    .child(div().flex_shrink_0().child(value.clone()))
                            }))
                    }),
                ));
            }
        } else if segments.is_empty() {
            details = details.child(
                div()
                    .text_xs()
                    .text_color(rgb(muted))
                    .child(self.text("此任务暂无分段信息", "No segment details for this task")),
            );
        }
        let visible_segments = if self.expanded { segments } else { &[] };
        for (index, segment) in visible_segments.iter().enumerate() {
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
        use gpui_motion_kit::core::{Motion, Target, Transition};
        let presentation = self.bar_motion.frame(
            Motion::new(
                Target::default().width(ratio.unwrap_or(if state == TaskState::Running {
                    1.
                } else {
                    0.
                })),
            )
            .transition(
                if state == TaskState::Running && window.is_window_active() && window.is_visible() {
                    crate::motion::tween(160)
                } else {
                    Transition::Instant
                },
            ),
            window,
            cx,
        );
        let fill = div()
            .h_full()
            .w(relative(
                presentation.target().width_px().unwrap_or(0.).clamp(0., 1.),
            ))
            .rounded_full()
            .bg(linear_gradient(
                90.,
                linear_color_stop(rgb(tone.rgb(0x885dff, 0x885dff)), 0.),
                linear_color_stop(
                    rgb(if state == TaskState::Completed {
                        0x3ed4b2
                    } else {
                        tone.rgb(0x37c9f5, 0x37c9f5)
                    }),
                    1.,
                ),
            ));
        let fill = fill.opacity(
            if ratio.is_none() && state == TaskState::Running && !cx.reduce_motion() {
                0.6 + 0.2 * (self.bar_epoch.elapsed().as_secs_f32() * 1.8).sin()
            } else {
                1.
            },
        );
        let mut content = div()
            .flex_1()
            .min_h_0()
            .px_4()
            .pt_1()
            .pb_3()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
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
            .child(progress_track(fill, edge));
        let elapsed = format_elapsed(self.snapshot.as_ref().map(|snapshot| snapshot.elapsed_ms));
        let mut chips = metric_group();
        if active {
            let speed_label = match phase {
                ProgressPhase::Downloading => self.text("下载速度", "Download speed"),
                ProgressPhase::Merging => self.text("合并速度", "Merge speed"),
                ProgressPhase::Saving => self.text("保存速度", "Save speed"),
                ProgressPhase::Finalizing => self.text("封装进度", "Remux progress"),
            };
            chips = chips
                .child(metric_chip(
                    IconName::ArrowDown,
                    speed_label,
                    if phase == ProgressPhase::Finalizing {
                        self.text("封装", "Remux").into()
                    } else if state == TaskState::Running {
                        format!("{}/s", format_bytes(self.speed))
                    } else {
                        "—".into()
                    },
                    edge,
                    muted,
                ))
                .child(metric_chip(
                    gpui_kit::assets::IconName::Clock,
                    self.text("预计剩余", "Time remaining"),
                    remaining,
                    edge,
                    muted,
                ))
                .child(metric_chip(
                    IconName::Globe,
                    if peers.is_some() {
                        self.text("活动连接", "Active connections")
                    } else {
                        use ndm_core::HttpRangeMode::*;
                        match self
                            .snapshot
                            .as_ref()
                            .and_then(|p| p.http.as_ref()?.range_mode)
                        {
                            Some(ConnectionLimit) => self.text(
                                "活动分段（任务连接数上限为 1）",
                                "Active segments (task limit is 1)",
                            ),
                            Some(RangeIgnored | RangeRejected | InvalidContentRange) => self.text(
                                "活动分段（服务器未返回有效范围）",
                                "Active segments (server did not return a valid range)",
                            ),
                            Some(MissingValidator) => self.text(
                                "活动分段（缺少可验证的文件版本或摘要）",
                                "Active segments (no verified file version or digest)",
                            ),
                            Some(SegmentFallback) => self.text(
                                "活动分段（分段响应未通过校验）",
                                "Active segments (range response failed validation)",
                            ),
                            Some(RateLimited) => self.text(
                                "活动分段（服务器限制并发）",
                                "Active segments (server rate limit)",
                            ),
                            _ => self.text("活动分段", "Active segments"),
                        }
                    },
                    if let Some(peers) = peers {
                        peers.connected.to_string()
                    } else if segments.is_empty() {
                        "—".into()
                    } else {
                        active_segments.to_string()
                    },
                    edge,
                    muted,
                ));
        }
        if let Some(peers) = peers {
            if sharing.is_some() {
                chips = chips
                    .child(metric_chip(
                        IconName::Globe,
                        self.text("已连接节点", "Connected peers"),
                        peers.connected.to_string(),
                        edge,
                        muted,
                    ))
                    .child(metric_chip(
                        IconName::ArrowUp,
                        self.text("上传速度", "Upload speed"),
                        format!("{}/s", format_bytes(peers.upload_bytes_per_second)),
                        edge,
                        muted,
                    ));
            }
            chips = chips.child(metric_chip(
                IconName::ChartPie,
                self.text("分享率", "Share ratio"),
                crate::peer_metrics::ratio(peers, downloaded),
                edge,
                muted,
            ));
        }
        chips = chips.child(metric_chip(
            gpui_kit::assets::IconName::Clock,
            self.text("总耗时", "Elapsed time"),
            elapsed,
            edge,
            muted,
        ));
        let peak = self.history.iter().copied().max().unwrap_or(1).max(1);
        let mut graph = div().ml_auto().h(px(16.)).flex().items_end().gap(px(2.));
        if state == TaskState::Running {
            for value in &self.history {
                graph = graph.child(
                    div()
                        .w(px(2.))
                        .h(px(2. + 12. * (*value as f32 / peak as f32)))
                        .rounded_full()
                        .bg(rgb(accent)),
                );
            }
        }
        let actions = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .when_some(task.as_ref(), |row, task| {
                row.child(chrome::copy_link_button(
                    "progress-copy-link",
                    task.request.url.clone(),
                    self.text("复制下载链接", "Copy download link"),
                ))
            })
            .when_some(action, |row, (icon, label)| {
                row.child(
                    chrome::icon_button("progress-action", icon, label)
                        .disabled(self.proxy_switching)
                        .on_click(cx.listener(move |this, _, _, cx| this.change_state(state, cx))),
                )
            })
            .when(can_share, |row| {
                let (icon, label) = if sharing.is_some() {
                    (IconName::Square, self.text("停止做种", "Stop seeding"))
                } else {
                    (IconName::Play, self.text("开始做种", "Start seeding"))
                };
                row.child(
                    chrome::icon_button("progress-seeding", icon, label)
                        .disabled(sharing == Some(ndm_core::SharingStatus::Stopping))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.change_sharing(sharing.is_none(), cx)
                        })),
                )
            })
            .child(
                chrome::icon_button(
                    "progress-folder",
                    IconName::FolderOpen,
                    self.text("打开文件夹", "Open folder"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.open_folder(cx))),
            )
            .when(state == TaskState::Completed, |row| {
                row.child(
                    chrome::icon_button(
                        "progress-file",
                        IconName::ExternalLink,
                        self.text("打开文件", "Open file"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.open_file(cx))),
                )
            })
            .when(
                matches!(
                    state,
                    TaskState::Queued | TaskState::Running | TaskState::Paused | TaskState::Failed
                ) && task.as_ref().is_some_and(|task| {
                    reqwest::Url::parse(&task.request.url)
                        .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
                }),
                |row| {
                    row.child(
                        chrome::icon_button(
                            "progress-proxy",
                            gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Zap),
                            self.text(
                                "仅对此任务使用已配置代理",
                                "Use configured proxy for this download only",
                            ),
                        )
                        .when(
                            task.as_ref().is_some_and(|task| match &task.options {
                                ndm_core::TaskOptions::Http(options) => options.proxy.is_some(),
                                ndm_core::TaskOptions::Hls(options) => options.proxy.is_some(),
                                ndm_core::TaskOptions::Dash(options) => options.proxy.is_some(),
                                _ => false,
                            }),
                            |button| button.primary(),
                        )
                        .disabled(self.proxy_switching)
                        .tooltip(if self.proxy_switching {
                            self.text("正在切换代理…", "Switching proxy…")
                        } else {
                            self.text(
                                "仅对此任务使用已配置代理",
                                "Use configured proxy for this download only",
                            )
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.enable_task_proxy(cx))),
                    )
                },
            )
            .when_some(self.owner.clone(), |row, owner| {
                let id = self.id;
                let label = self.text("删除任务、缓存及文件", "Delete task, cache and file");
                let confirm = self.text("确认删除任务及文件", "Confirm deletion of task and file");
                row.child(
                    chrome::icon_button(
                        "progress-delete",
                        gpui_kit::assets::IconName::Trash,
                        label,
                    )
                    .text_color(rgb(tone.rgb(0xf28b9c, 0xbe3049)))
                    .disabled(self.proxy_switching)
                    .dropdown_menu(move |menu, _, _| {
                        let owner = owner.clone();
                        menu.item(
                            PopupMenuItem::element(move |_, _| {
                                div()
                                    .text_sm()
                                    .text_color(rgb(tone.rgb(0xf28b9c, 0xbe3049)))
                                    .child(confirm)
                            })
                            .on_click(move |_, _, cx| {
                                let _ = owner
                                    .update(cx, |this, cx| this.delete_tasks(vec![id], true, cx));
                            }),
                        )
                    }),
                )
            })
            .child(
                chrome::icon_button(
                    "progress-details",
                    if self.expanded {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    },
                    self.text("展开／收起详情", "Expand / collapse details"),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.expanded = !this.expanded;
                    cx.notify();
                })),
            );
        content = content.child(compact_footer(chips, actions));
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
                    .child(graph)
                    .child(details),
            );
        }
        chrome::blank_drag_region("download-progress", window, cx)
            .size_full()
            .flex()
            .flex_col()
            .bg(chrome::floating_surface(tone))
            .text_color(rgb(fg))
            .child(chrome::floating_header(
                "OnesDM",
                tone,
                self.language == Language::Chinese,
                true,
            ))
            .child(content)
    }
}

fn metric_group() -> Div {
    div().flex().flex_shrink_0().items_center().gap(px(4.))
}

fn compact_footer(metrics: impl IntoElement, actions: impl IntoElement) -> Stateful<Div> {
    div()
        .id("progress-footer")
        .debug_selector(|| "progress-footer".into())
        .flex()
        .items_center()
        .h(px(28.))
        .flex_shrink_0()
        .gap_2()
        .child(metrics)
        .child(div().ml_auto().flex_shrink_0().child(actions))
}

fn progress_track(fill: impl IntoElement, edge: u32) -> Stateful<Div> {
    div()
        .id("progress-track")
        .w_full()
        .h(px(4.))
        .flex_shrink_0()
        .rounded_full()
        .overflow_hidden()
        .bg(rgb(edge))
        .child(fill)
}

fn metric_chip(
    icon: impl Into<gpui_kit::component::Icon>,
    label: &'static str,
    value: String,
    edge: u32,
    muted: u32,
) -> Stateful<Div> {
    use gpui_kit::component::tooltip::Tooltip;
    let tooltip = format!("{label}: {value}");
    div()
        .id(SharedString::from(format!("metric-{label}")))
        .h(px(28.))
        .flex_shrink_0()
        .px(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .rounded_md()
        .border_1()
        .border_color(rgb(edge))
        .tooltip(move |_, cx| cx.new(|_| Tooltip::new(tooltip.clone())).into())
        .child(
            div()
                .debug_selector(move || format!("metric-{label}-icon"))
                .size(px(12.))
                .flex_shrink_0()
                .child(icon.into().with_size(px(12.)).text_color(rgb(muted))),
        )
        .child(
            div()
                .debug_selector(move || format!("metric-{label}-value"))
                .min_w_0()
                .text_size(px(11.))
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
}

#[cfg(test)]
mod tests {
    use super::{compact_footer, metric_chip, metric_group, progress_track};
    use gpui::{
        Context, InteractiveElement, IntoElement, Modifiers, ParentElement, Render, Styled, Window,
        div, prelude::FluentBuilder, px,
    };
    use gpui_kit as gpui;
    use std::{cell::Cell, rc::Rc};

    #[gpui::test]
    fn compact_progress_keeps_all_metrics_and_actions_on_one_clickable_row(
        cx: &mut gpui::TestAppContext,
    ) {
        struct Footer(Rc<Cell<usize>>, bool);
        impl Render for Footer {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let mut metrics = metric_group();
                let values = if self.1 {
                    [
                        ("speed", "10.7 MB/s"),
                        ("eta", "33s"),
                        ("peers", "16/16"),
                        ("elapsed", "00:02"),
                    ]
                } else {
                    [
                        ("speed", "999.9 MB/s"),
                        ("eta", "12h 59m"),
                        ("peers", "16/16"),
                        ("elapsed", "100:59:59"),
                    ]
                };
                for (label, value) in values {
                    metrics = metrics.child(
                        metric_chip(gpui::assets::IconName::Clock, label, value.into(), 0, 0)
                            .debug_selector(move || label.into()),
                    );
                }
                let mut actions = div().flex().flex_shrink_0();
                for (id, icon) in [
                    ("copy", gpui::assets::IconName::Copy),
                    ("pause", gpui::assets::IconName::Pause),
                    ("folder", gpui::assets::IconName::FolderOpen),
                    ("proxy", gpui::assets::IconName::Zap),
                    ("delete", gpui::assets::IconName::Trash),
                    ("details", gpui::assets::IconName::ChevronDown),
                ] {
                    let clicks = self.0.clone();
                    actions = actions.child(
                        crate::chrome::icon_button(id, icon, id)
                            .debug_selector(move || id.into())
                            .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                    );
                }
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().h(px(34.)).flex_shrink_0())
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .px_4()
                            .pt_1()
                            .pb_3()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(div().h(px(40.)).flex_shrink_0())
                            .child(progress_track(div(), 0))
                            .child(compact_footer(metrics, actions)),
                    )
            }
        }
        cx.update(gpui::init);
        let clicks = Rc::new(Cell::new(0));
        let (view, cx) = cx.add_window_view({
            let clicks = clicks.clone();
            move |_, _| Footer(clicks, false)
        });
        for (scale, compact) in [1., 1.5, 2.]
            .into_iter()
            .flat_map(|scale| [false, true].map(move |compact| (scale, compact)))
        {
            cx.simulate_scale_factor_change(scale);
            cx.simulate_resize(gpui::size(
                px(super::super::WINDOW_WIDTH),
                px(super::super::COMPACT_HEIGHT),
            ));
            cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.1 = compact;
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let row = cx.debug_bounds("progress-footer").unwrap();
            assert!(row.bottom() <= px(super::super::COMPACT_HEIGHT - 12.));
            let mut last = row.left();
            for id in [
                "speed", "eta", "peers", "elapsed", "copy", "pause", "folder", "proxy", "delete",
                "details",
            ] {
                let item = cx.debug_bounds(id).unwrap();
                assert_eq!(item.top(), row.top(), "{id}");
                assert!(item.left() >= last, "{id} overlaps");
                assert!(item.right() <= row.right(), "{id} is clipped");
                assert!(item.size.width >= px(28.), "{id} collapsed");
                last = item.right();
            }
            for (label, icon_id, value_id) in [
                ("speed", "metric-speed-icon", "metric-speed-value"),
                ("eta", "metric-eta-icon", "metric-eta-value"),
                ("peers", "metric-peers-icon", "metric-peers-value"),
                ("elapsed", "metric-elapsed-icon", "metric-elapsed-value"),
            ] {
                let chip = cx.debug_bounds(label).unwrap();
                let icon = cx.debug_bounds(icon_id).unwrap();
                let value = cx.debug_bounds(value_id).unwrap();
                let left_inset = icon.left() - chip.left();
                let right_inset = chip.right() - value.right();
                assert!(
                    (px(6.)..=px(8.)).contains(&left_inset),
                    "{label}: left inset {left_inset:?}"
                );
                assert!(
                    (px(6.)..=px(8.)).contains(&right_inset),
                    "{label}: right inset {right_inset:?}"
                );
            }
            let gap = cx.debug_bounds("copy").unwrap().left()
                - cx.debug_bounds("elapsed").unwrap().right();
            assert!(gap >= px(8.), "Metrics overlap actions: {gap:?}");
            assert_eq!(cx.debug_bounds("details").unwrap().right(), row.right());
            for id in ["copy", "pause", "folder", "proxy", "delete", "details"] {
                let before = clicks.get();
                let center = cx.debug_bounds(id).unwrap().center();
                cx.simulate_click(center, Modifiers::default());
                assert_eq!(clicks.get(), before + 1);
            }
        }
    }

    #[gpui::test]
    fn progress_track_keeps_its_height_in_compact_and_expanded_layouts(
        cx: &mut gpui::TestAppContext,
    ) {
        struct Harness(bool);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(300.))
                    .h(px(if self.0 { 300. } else { 64. }))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().h(px(42.)))
                    .child(
                        progress_track(div().size_full(), 0)
                            .debug_selector(|| "progress-track".into()),
                    )
                    .child(div().h(px(28.)))
                    .when(self.0, |body| body.child(div().flex_1().min_h_0()))
            }
        }
        cx.update(gpui::init);
        let (view, cx) = cx.add_window_view(|_, _| Harness(false));
        for expanded in [false, true, false] {
            cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.0 = expanded;
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            assert_eq!(
                cx.debug_bounds("progress-track").unwrap().size.height,
                px(4.)
            );
        }
    }
}
