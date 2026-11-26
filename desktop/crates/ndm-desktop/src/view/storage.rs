use super::*;
use gpui_kit::component::Disableable;

impl Desktop {
    pub(super) fn storage_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = crate::motion::ThemeTone::current(cx).rgb(0x95a5c0, 0x71809a);
        let report = self
            .cache_report
            .map(|report| {
                self.preferences.language.format(
                    "缓存 {size} · 可清理 {free} · 保留 {kept}",
                    "Cache {size} · reclaimable {free} · protected {kept}",
                    &[
                        ("size", &format_bytes(report.bytes)),
                        ("free", &format_bytes(report.reclaimable_bytes)),
                        ("kept", &format_bytes(report.protected_bytes)),
                    ],
                )
            })
            .unwrap_or_else(|| {
                self.text(
                    "统计下载临时文件及更新安装包",
                    "Download temporary files and update packages",
                )
                .into()
            });
        div().flex().flex_col().gap_3()
            .child(self.storage_directory_field(false, cx))
            .child(self.storage_directory_field(true, cx))
            .child(div().flex().items_center().gap_2()
                .child(div().text_sm().child(self.text("闲置缓存保留天数", "Unused cache retention (days)")))
                .child(div().w(px(72.)).child(Input::new(&self.retention_input).small().disabled(self.storage_busy)))
                .child(div().text_xs().text_color(rgb(muted)).child(self.text("0 关闭 · 默认 7 天", "0 disables · default 7 days"))))
            .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                "仅新任务使用新位置；清理保留未完成任务的续传数据和待安装更新。",
                "New tasks use the new location. Cleanup keeps resumable tasks and pending updates.")))
            .child(div().flex().items_center().gap_2()
                .child(Button::new("storage-save").small().primary().disabled(self.storage_busy)
                    .label(self.text("保存", "Save"))
                    .on_click(cx.listener(|this, _, window, cx| this.save_storage_settings(window, cx))))
                .child(Button::new("cache-clean").small().disabled(self.storage_busy)
                    .label(self.text("清理缓存", "Clear cache"))
                    .on_click(cx.listener(|this, _, _, cx| this.maintain_storage(true, false, cx))))
                .child(Button::new("cache-refresh").small().ghost().disabled(self.storage_busy)
                    .label(self.text("刷新统计", "Refresh"))
                    .on_click(cx.listener(|this, _, _, cx| this.maintain_storage(false, false, cx))))
                .child(div().flex_1().min_w_0().truncate().text_xs().text_color(rgb(muted))
                    .child(if self.storage_busy { self.text("正在处理…", "Working…").to_owned() } else { report })))
            .into_any_element()
    }

    fn storage_directory_field(&self, cache: bool, cx: &mut Context<Self>) -> AnyElement {
        let input = if cache {
            &self.cache_input
        } else {
            &self.storage_download_input
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child(if cache {
                self.text("缓存目录", "Cache directory")
            } else {
                self.text("默认下载目录", "Default download directory")
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(input).small().disabled(self.storage_busy)),
                    )
                    .child(
                        Button::new(if cache {
                            "cache-browse"
                        } else {
                            "downloads-browse"
                        })
                        .small()
                        .icon(IconName::Folder)
                        .label(self.text("选择", "Choose"))
                        .disabled(self.storage_busy)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.choose_storage_directory(cache, window, cx)
                            },
                        )),
                    )
                    .child(
                        Button::new(if cache {
                            "cache-reset"
                        } else {
                            "downloads-reset"
                        })
                        .small()
                        .ghost()
                        .label(if cache {
                            self.text("默认", "Default")
                        } else {
                            self.text("系统目录", "System folder")
                        })
                        .disabled(self.storage_busy)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                let path = if cache {
                                    ndm_core::default_cache_directory()
                                } else {
                                    ndm_core::system_download_directory()
                                };
                                let input = if cache {
                                    &this.cache_input
                                } else {
                                    &this.storage_download_input
                                };
                                input.update(cx, |input, cx| {
                                    input.set_value(path.to_string_lossy().into_owned(), window, cx)
                                });
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    fn choose_storage_directory(
        &mut self,
        cache: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.text("选择文件夹", "Choose folder").into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = view.update_in(cx, |this, window, cx| {
                    let input = if cache {
                        &this.cache_input
                    } else {
                        &this.storage_download_input
                    };
                    input.update(cx, |input, cx| {
                        input.set_value(path.to_string_lossy().into_owned(), window, cx)
                    });
                });
            }
        })
        .detach();
    }

    fn save_storage_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.storage_busy {
            return;
        }
        let downloads = PathBuf::from(self.storage_download_input.read(cx).value().trim());
        let cache = PathBuf::from(self.cache_input.read(cx).value().trim());
        let Ok(days) = self.retention_input.read(cx).value().parse::<u32>() else {
            self.set_notice(
                self.text("保留天数应为 0–3650。", "Retention must be 0–3650 days.")
                    .into(),
                cx,
            );
            return;
        };
        self.storage_busy = true;
        let runtime = self.runtime.clone();
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            let outcome = runtime.spawn_blocking(move || crate::storage::prepare_settings(downloads, cache, days)).await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.storage_busy = false;
                let result = match outcome {
                    Ok(Ok(choice)) => crate::storage::apply_settings(this.preferences.clone(), &this.preferences_path, choice),
                    _ => Err(std::io::Error::other("invalid storage settings")),
                };
                match result {
                    Ok(preferences) => {
                        this.preferences = preferences;
                        this.scheduler.update_options(this.preferences.scheduler_options());
                        this.directory_input.update(cx, |input, cx| input.set_value(this.preferences.download_dir.to_string_lossy().into_owned(), window, cx));
                        this.cache_report = None;
                        this.set_notice(this.text("存储设置已保存。", "Storage settings saved.").into(), cx);
                        this.maintain_storage(false, false, cx);
                    }
                    Err(_) => this.set_notice(this.text("无法保存：请选择可写的绝对目录，保留天数为 0–3650。", "Could not save. Choose writable absolute folders and 0–3650 retention days.").into(), cx),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn maintain_storage(
        &mut self,
        remove: bool,
        automatic: bool,
        cx: &mut Context<Self>,
    ) {
        if self.storage_busy {
            return;
        }
        self.storage_busy = true;
        if remove {
            self.preferences.last_cache_cleanup = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
            self.save_preferences(cx);
        }
        let (runtime, scheduler, preferences, path, gate) = (
            self.runtime.clone(),
            self.scheduler.clone(),
            self.preferences.clone(),
            self.preferences_path.clone(),
            self.update_cache_gate.clone(),
        );
        cx.notify();
        cx.spawn(async move |view, cx| {
            let outcome = runtime
                .spawn_blocking(move || {
                    crate::storage::maintain(
                        &scheduler,
                        &preferences,
                        &path,
                        &gate,
                        automatic,
                        remove,
                    )
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.storage_busy = false;
                match outcome {
                    Ok(Ok(report)) => {
                        this.cache_report = Some(report);
                        if remove && !automatic {
                            let language = this.preferences.language;
                            let mut message = language.format(
                                "已清理 {freed}，保留 {kept} 续传及更新数据。",
                                "Freed {freed}. Kept {kept} of resumable data and updates.",
                                &[
                                    ("freed", &format_bytes(report.removed_bytes)),
                                    ("kept", &format_bytes(report.protected_bytes)),
                                ],
                            );
                            if report.skipped_directories > 0 {
                                message.push(' ');
                                message.push_str(&language.format(
                                    "跳过 {count} 个无法安全清理的目录。",
                                    "Skipped {count} unsafe or inaccessible folders.",
                                    &[("count", &report.skipped_directories.to_string())],
                                ));
                            }
                            this.set_notice(message, cx);
                        }
                    }
                    _ if !automatic => this.set_notice(
                        this.text("无法读取或清理缓存。", "Could not read or clean the cache.")
                            .into(),
                        cx,
                    ),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_storage_watcher(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(30))
                .await;
            loop {
                if view
                    .update(cx, |this, cx| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if this.preferences.cache_retention_days > 0
                            && crate::storage::cleanup_due(this.preferences.last_cache_cleanup, now)
                        {
                            this.maintain_storage(true, true, cx);
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
}
