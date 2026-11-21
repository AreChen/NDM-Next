use super::*;
use gpui_kit::component::{Disableable, menu::PopupMenuItem};

impl Desktop {
    fn change_sharing(&mut self, id: TaskId, start: bool, cx: &mut Context<Self>) {
        if matches!(self.update_state, UpdateState::Installing) {
            return;
        }
        let scheduler = self.scheduler.clone();
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            if !matches!(
                runtime
                    .spawn(async move { scheduler.set_sharing(id, start).await })
                    .await,
                Ok(Ok(()))
            ) {
                let _ = view.update(cx, |this, cx| {
                    this.set_notice(this.text(
                    "无法开始做种。请检查文件或停止其他做种任务。",
                    "Could not start seeding. Check the files or stop another seeding task.",
                ).into(), cx)
                });
            }
        })
        .detach();
    }

    fn download_again(&mut self, id: TaskId, cx: &mut Context<Self>) {
        if matches!(self.update_state, UpdateState::Installing) {
            return;
        }
        let store = self.store.clone();
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let result = runtime
                .spawn_blocking(move || crate::redownload::enqueue(&store, id))
                .await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(Ok(task)) => {
                        this.seen_tasks.insert(task.id);
                        this.scheduler.notify();
                        this.reload();
                        this.show_progress(task.id, cx);
                    }
                    _ => this.set_notice(
                        this.text(
                            "无法重新下载，请检查链接是否仍然有效。",
                            "Could not download again. Check that the link is still valid.",
                        )
                        .into(),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn task_row(
        &self,
        task: &Task,
        _dark: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = task.id;
        let state = task.state;
        let sharing = self.scheduler.sharing_status(id);
        let can_share = self.scheduler.can_share(task);
        let name = crate::filenames::display_name(task)
            .unwrap_or_else(|| self.text("未命名下载", "Untitled download").into());
        let host = reqwest::Url::parse(&task.request.url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default();
        let progress = self
            .scheduler
            .progress(id)
            .or_else(|| self.retained_progress.get(&id).cloned())
            .or_else(|| task.progress.clone());
        let mut total = self.task_index.known_size(id).or_else(|| {
            progress
                .as_ref()
                .and_then(|value| value.total)
                .or(task.request.expected_size)
        });
        if state == TaskState::Running
            && progress
                .as_ref()
                .is_some_and(|value| value.phase == ndm_core::ProgressPhase::Saving)
        {
            total = progress.as_ref().and_then(|value| value.save_total);
        }
        let done = progress
            .as_ref()
            .map(|value| {
                if state == TaskState::Running && value.phase == ndm_core::ProgressPhase::Saving {
                    value.saved
                } else if state == TaskState::Running
                    && value.phase == ndm_core::ProgressPhase::Merging
                {
                    value.merged
                } else {
                    value.downloaded
                }
            })
            .unwrap_or(0);
        let ratio = if state == TaskState::Completed {
            Some(1.)
        } else if state == TaskState::Running
            && progress
                .as_ref()
                .is_some_and(|value| value.phase == ndm_core::ProgressPhase::Finalizing)
        {
            progress.as_ref().and_then(|value| {
                value
                    .finalization_total_ms
                    .filter(|total| *total > 0)
                    .map(|total| (value.finalized_ms as f32 / total as f32).clamp(0., 0.999))
            })
        } else if progress.is_some() || state == TaskState::Queued {
            total
                .filter(|value| *value > 0)
                .map(|value| (done as f32 / value as f32).clamp(0., 0.999))
        } else {
            None
        };
        let zh = self.preferences.language == Language::Chinese;
        let status = match state {
            TaskState::Prepared => self.text("待确认", "Pending"),
            TaskState::Queued => self.text("等待中", "Queued"),
            TaskState::Running => match progress.as_ref().map(|p| p.phase) {
                Some(ndm_core::ProgressPhase::Saving) => self.text("保存中", "Saving"),
                Some(ndm_core::ProgressPhase::Merging) => self.text("合并中", "Merging"),
                Some(ndm_core::ProgressPhase::Finalizing) => self.text("封装中", "Finalizing"),
                _ => self.text("下载中", "Downloading"),
            },
            TaskState::Paused => self.text("已暂停", "Paused"),
            TaskState::Completed => sharing.map_or(self.text("已完成", "Completed"), |status| {
                crate::peer_metrics::status(status, zh)
            }),
            TaskState::Failed => self.text("失败", "Failed"),
        };
        let palette = |dark| match (state, dark) {
            (TaskState::Running, true) => (0x252044, 0x183e69, 0x49418c, 0xb19bff),
            (TaskState::Completed, true) => (0x172238, 0x124b40, 0x286055, 0x60e1bc),
            (TaskState::Paused, true) => (0x222435, 0x443424, 0x62513c, 0xe7ba6f),
            (TaskState::Failed, true) => (0x252237, 0x4c2836, 0x764052, 0xff929d),
            (_, true) => (0x1a2134, 0x1c2b42, 0x303e59, 0x9facca),
            (TaskState::Running, false) => (0xf0ebff, 0xdeefff, 0xc6c0f2, 0x6e4dcc),
            (TaskState::Completed, false) => (0xf3f8fc, 0xdaf4e9, 0xb4dccc, 0x168262),
            (TaskState::Paused, false) => (0xf8f7fc, 0xffefd7, 0xe7d3af, 0x9a651d),
            (TaskState::Failed, false) => (0xf8f4fc, 0xffe2e8, 0xeabdc7, 0xb93752),
            (_, false) => (0xffffff, 0xedf3fc, 0xdce4f1, 0x697c9c),
        };
        let tone = crate::motion::ThemeTone::current(cx);
        let d = palette(true);
        let l = palette(false);
        let (start, end, edge, accent) = (
            tone.rgb(d.0, l.0),
            tone.rgb(d.1, l.1),
            tone.rgb(d.2, l.2),
            tone.rgb(d.3, l.3),
        );
        let foreground = tone.rgb(0xf1f3ff, 0x22324b);
        let muted = tone.rgb(0xa0b0cc, 0x6c7b94);
        let size = total
            .map(format_bytes)
            .unwrap_or_else(|| self.text("大小未知", "Unknown size").into());
        let speed = self.speed.get(&id).copied().unwrap_or(0);
        let error = task.error.as_ref().map(|code| {
            crate::failure::description(code, self.preferences.language == Language::Chinese)
                .to_owned()
        });
        let pending_key = self.pending_host_keys.contains_key(&id);
        let is_hls = matches!(task.options, TaskOptions::Hls(_))
            || reqwest::Url::parse(&task.request.url)
                .is_ok_and(|url| url.path().ends_with(".m3u8"));
        let can_edit = matches!(state, TaskState::Paused | TaskState::Failed)
            && reqwest::Url::parse(&task.request.url)
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
        let download_url = task.request.url.clone();
        let view = cx.entity().downgrade();
        let more = crate::chrome::icon_button(
            SharedString::from(format!("more-task-{id}")),
            IconName::Ellipsis,
            self.text("更多操作", "More actions"),
        )
        .dropdown_menu(move |mut menu, _, _| {
            let url = download_url.clone();
            menu = menu.item(
                PopupMenuItem::new(if zh {
                    "复制下载链接"
                } else {
                    "Copy download link"
                })
                .icon(gpui_kit::component::Icon::new(
                    gpui_kit::assets::IconName::Copy,
                ))
                .on_click(move |_, _, cx| crate::chrome::copy_download_link(&url, cx)),
            );
            if state != TaskState::Prepared {
                let view = view.clone();
                menu = menu.item(
                    PopupMenuItem::new(if zh { "再次下载" } else { "Download again" })
                        .icon(gpui_kit::component::Icon::new(
                            gpui_kit::assets::IconName::RotateCw,
                        ))
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| this.download_again(id, cx));
                        }),
                );
            }
            if let Some(error) = &error {
                menu = menu.item(PopupMenuItem::new(error.clone()).disabled(true));
            }
            if can_edit {
                let view = view.clone();
                menu = menu.item(
                    PopupMenuItem::new(if zh {
                        "更换下载链接"
                    } else {
                        "Change download link"
                    })
                    .on_click(move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| {
                            if matches!(this.update_state, UpdateState::Installing) {
                                return;
                            }
                            this.replacement_input
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            this.editing_url = Some(id);
                            cx.notify();
                        });
                    }),
                );
            }
            if state == TaskState::Running && is_hls {
                let view = view.clone();
                menu = menu.item(
                    PopupMenuItem::new(if zh {
                        "停止直播并保存"
                    } else {
                        "Stop and save live media"
                    })
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| {
                            if this.scheduler.stop_live(id).is_err() {
                                this.set_notice(
                                    this.text(
                                        "无法停止该媒体任务。",
                                        "Could not stop the media task.",
                                    )
                                    .into(),
                                    cx,
                                );
                            }
                            cx.notify();
                        });
                    }),
                );
            }
            if pending_key {
                let view = view.clone();
                menu = menu.item(
                    PopupMenuItem::new(if zh {
                        "核对 SFTP 主机密钥"
                    } else {
                        "Review SFTP host key"
                    })
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| {
                            if matches!(this.update_state, UpdateState::Installing) {
                                return;
                            }
                            this.confirming_host_key = Some(id);
                            cx.notify();
                        });
                    }),
                );
            }
            if state != TaskState::Prepared {
                let view = view.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new(if zh {
                        "删除任务…"
                    } else {
                        "Delete download…"
                    })
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| this.confirm_delete(vec![id], cx));
                    }),
                );
            }
            menu
        });
        let action = match state {
            TaskState::Queued | TaskState::Running => {
                Some((IconName::Pause, self.text("暂停", "Pause")))
            }
            TaskState::Paused => Some((IconName::Play, self.text("继续", "Resume"))),
            TaskState::Failed => Some((IconName::RotateCw, self.text("重试", "Retry"))),
            TaskState::Completed if can_share => Some(if sharing.is_some() {
                (IconName::Square, self.text("停止做种", "Stop seeding"))
            } else {
                (IconName::Play, self.text("开始做种", "Start seeding"))
            }),
            _ => None,
        };
        let mut metrics = div()
            .w(px(178.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(4.));
        if state == TaskState::Running || state == TaskState::Paused || state == TaskState::Queued {
            metrics = metrics
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_xs()
                        .child(
                            ratio
                                .map(|value| format!("{}%", (value * 100.).floor() as u32))
                                .unwrap_or_else(|| "—".into()),
                        )
                        .child(div().text_color(rgb(muted)).child(
                            if state == TaskState::Running {
                                format!("{}/s", format_bytes(speed))
                            } else {
                                size.clone()
                            },
                        )),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(4.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(rgba(tone.rgba(0xffffff20, 0x26365c18)))
                        .child(
                            div()
                                .h_full()
                                .w(relative(ratio.unwrap_or(0.)))
                                .rounded_full()
                                .bg(linear_gradient(
                                    90.,
                                    linear_color_stop(rgb(0x8958ff), 0.),
                                    linear_color_stop(rgb(0x39cbf3), 1.),
                                )),
                        ),
                );
        } else {
            metrics = metrics.child(div().text_xs().text_color(rgb(accent)).child(
                if state == TaskState::Failed {
                    self.text("可重试或更换链接", "Retry or change link").into()
                } else if sharing == Some(ndm_core::SharingStatus::Sharing) {
                    let speed = progress
                        .as_ref()
                        .and_then(|progress| progress.peers)
                        .map_or(0, |peers| peers.upload_bytes_per_second);
                    format!("{} {}/s", self.text("上传", "Up"), format_bytes(speed))
                } else {
                    size.clone()
                },
            ));
        }
        if let Some(snapshot) = &progress
            && let Some(peers) = snapshot.peers
        {
            let details = crate::peer_metrics::details(peers, snapshot.downloaded, zh)
                .into_iter()
                .map(|(label, value)| format!("{label} {value}"))
                .collect::<Vec<_>>()
                .join("\n");
            let ratio = crate::peer_metrics::ratio(peers, snapshot.downloaded);
            metrics = metrics.child(
                Button::new(SharedString::from(format!("peer-metrics-{id}")))
                    .ghost()
                    .small()
                    .h(px(17.))
                    .px_0()
                    .label(format!(
                        "{} {} · {} {ratio}",
                        self.text("节点", "Peers"),
                        peers.connected,
                        self.text("分享率", "Ratio")
                    ))
                    .text_size(px(11.))
                    .text_color(rgb(muted))
                    .tooltip(details)
                    .on_click(cx.listener(move |this, _, _, cx| this.show_progress(id, cx))),
            );
        }
        div().h(px(58.)).pb(px(6.)).child(
            div()
                .id(SharedString::from(format!("task-selection-area-{id}")))
                .block_mouse_except_scroll()
                .cursor_pointer()
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.page_focus.focus(window, cx);
                    this.select_task(id, event.modifiers().shift, cx);
                }))
                .w_full()
                .h_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .rounded_lg()
                .border_1()
                .border_color(rgb(if self.selected_tasks.contains(&id) {
                    0x967bff
                } else {
                    edge
                }))
                .bg(linear_gradient(
                    105.,
                    linear_color_stop(rgb(start), 0.),
                    linear_color_stop(rgb(end), 1.),
                ))
                .child(
                    Button::new(SharedString::from(format!("select-task-{id}")))
                        .small()
                        .ghost()
                        .icon(if self.selected_tasks.contains(&id) {
                            IconName::CircleCheck
                        } else {
                            IconName::Square
                        })
                        .accessibility_label(self.text("选择任务", "Select download"))
                        .tooltip(self.text("选择 · Shift 连选", "Select · Shift for a range"))
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.select_task(id, event.modifiers().shift, cx);
                        })),
                )
                .child(
                    div()
                        .size(px(28.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .bg(rgba(tone.rgba(0x8a66ff25, 0x8a66ff18)))
                        .text_color(rgb(accent))
                        .child(Icon::new(IconName::File).with_size(px(18.))),
                )
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
                                .text_color(rgb(foreground))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(muted))
                                .truncate()
                                .child(format!("{host}  ·  {size}")),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.))
                        .px_2()
                        .py(px(3.))
                        .rounded_full()
                        .text_color(rgb(accent))
                        .bg(rgba(tone.rgba(0xffffff0d, 0xffffff80)))
                        .child(status),
                )
                .child(metrics)
                .child(
                    crate::chrome::task_actions(SharedString::from(format!("task-actions-{id}")))
                        .when_some(action, |row, (icon, label)| {
                            row.child(
                                crate::chrome::icon_button(
                                    SharedString::from(format!("task-action-{id}")),
                                    icon,
                                    label,
                                )
                                .disabled(sharing == Some(ndm_core::SharingStatus::Stopping))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if can_share {
                                            this.change_sharing(id, sharing.is_none(), cx);
                                        } else {
                                            this.task_action(id, state);
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                        })
                        .child(
                            crate::chrome::icon_button(
                                SharedString::from(format!("task-progress-{id}")),
                                IconName::ChartPie,
                                self.text("下载详情", "Download details"),
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.show_progress(id, cx)),
                            ),
                        )
                        .child(
                            crate::chrome::icon_button(
                                SharedString::from(format!("task-folder-{id}")),
                                IconName::FolderOpen,
                                self.text("打开文件夹", "Open folder"),
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.open_task_path(id, false, cx)
                                }),
                            ),
                        )
                        .when(state == TaskState::Completed, |row| {
                            row.child(
                                crate::chrome::icon_button(
                                    SharedString::from(format!("task-open-{id}")),
                                    IconName::ExternalLink,
                                    self.text("打开文件", "Open file"),
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.open_task_path(id, true, cx),
                                )),
                            )
                        })
                        .child(more),
                ),
        )
    }
}
