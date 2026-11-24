use super::*;
use crate::motion::{SegmentedControl, Switch};
use gpui_motion_kit::{
    Frame, MotionState,
    core::{Motion, Target},
};

const CHROME_EXTENSION_URL: &str =
    "https://chromewebstore.google.com/detail/ndm-next/caegebfdojcdlkkiaiokeoadefdooamd";
const EDGE_EXTENSION_URL: &str =
    "https://microsoftedge.microsoft.com/addons/detail/ndm-next/hiblommiheggidolemmnnlocmocebckp";
const EXTENSION_GUIDE_URL: &str =
    "https://github.com/AreChen/NDM-Next/blob/main/docs/extension-install.md";
const USER_AGENT_PRESETS: [(&str, &str); 3] = [
    ("Chrome · Windows", ndm_core::CHROME_USER_AGENT),
    (
        "Firefox · Windows",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:145.0) Gecko/20100101 Firefox/145.0",
    ),
    (
        "Safari · macOS",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Safari/605.1.15",
    ),
];

impl Desktop {
    pub(super) fn toggle_settings_group(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !toggle_group(&mut self.settings_open, index) {
            self.page_focus.focus(window, cx);
        } else if index == 6 {
            self.maintain_storage(false, false, cx);
        } else if index == plugins::AI_CLIENTS_GROUP {
            self.refresh_ai_client_status(cx);
        }
        cx.notify();
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        if self.preferences.language == language {
            return;
        }
        self.clear_notice();
        self.preferences.language = language;
        self.language_motion = (!cx.reduce_motion()).then(|| gpui_motion_kit::ValueState::new(0.));
        self.language_progress = if self.language_motion.is_some() {
            0.
        } else {
            1.
        };
        let placeholder = self.text("搜索", "Search");
        self.search_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx)
        });
        self.market_search_input.update(cx, |input, cx| {
            input.set_placeholder(plugins::market_search_placeholder(language), window, cx)
        });
        #[cfg(any(windows, target_os = "macos"))]
        if let Some(tray) = &self.tray {
            tray.set_language(language);
        }
        for (_, progress) in self.progress_windows.values() {
            if let Some(progress) = progress.upgrade() {
                progress.update(cx, |view, cx| {
                    view.language = language;
                    cx.notify();
                });
            }
        }
        self.save_preferences(cx);
        cx.notify();
    }

    fn save_user_agent(&mut self, cx: &mut Context<Self>) {
        let value = self.user_agent_input.read(cx).value();
        let value = value.trim();
        if !value.is_empty() && ndm_core::validate_user_agent(value).is_err() {
            self.set_notice(
                self.text(
                    "User-Agent 须为 1–1024 个可见 ASCII 字符或空格。留空可恢复默认。",
                    "Use 1–1024 printable ASCII characters, or leave the User-Agent blank for the default.",
                )
                .into(),
                cx,
            );
            return;
        }
        let mut preferences = self.preferences.clone();
        preferences.user_agent = (!value.is_empty()).then(|| value.to_owned());
        if preferences.save(&self.preferences_path).is_err() {
            self.set_notice(
                self.text("无法保存设置。", "Could not save settings.")
                    .into(),
                cx,
            );
            return;
        }
        self.preferences = preferences;
        self.scheduler
            .update_options(self.preferences.scheduler_options());
        self.set_notice(
            self.text("User-Agent 已保存。", "User-Agent saved.").into(),
            cx,
        );
    }

    fn settings_group(
        &self,
        group: (usize, IconName, &'static str, &'static str),
        quick: impl IntoElement,
        detail: Option<AnyElement>,
        frame: Frame,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (index, icon, title, summary) = group;
        let tone = crate::motion::ThemeTone::current(cx);
        let open = self.settings_open.contains(&index);
        let has_detail = detail.is_some();
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .size(px(36.))
                    .flex_shrink_0()
                    .rounded_lg()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(tone.rgba(0x885dff28, 0x885dff16)))
                    .text_color(rgb(tone.rgb(0xc3b4ff, 0x6841cc)))
                    .child(Icon::new(icon).with_size(px(19.))),
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
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.display_text(title)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(tone.rgb(0x95a5c0, 0x71809a)))
                            .truncate()
                            .child(self.display_text(summary)),
                    ),
            )
            .when(has_detail, |header| {
                header.child(
                    crate::chrome::icon_button(
                        ("settings-section", index),
                        if open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        },
                        if open {
                            self.text("收起设置", "Collapse settings")
                        } else {
                            self.text("更多设置", "More settings")
                        },
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_settings_group(index, window, cx);
                    })),
                )
            });
        settings_card(
            index,
            header,
            quick,
            detail.map(|detail| settings_detail(index, frame, open, detail)),
            tone,
        )
        .into_any_element()
    }

    pub(super) fn settings_panel(
        &self,
        dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tone = crate::motion::ThemeTone::current(cx);
        let muted = tone.rgb(0x95a5c0, 0x71809a);
        let view = cx.entity().downgrade();
        let frames: [Frame; plugins::AI_CLIENTS_GROUP + 1] = std::array::from_fn(|index| {
            disclosure_frame(index, self.settings_open.contains(&index), window, cx)
        });
        let current_language = self.preferences.language;
        let appearance = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(theme_switch(
                self.preferences.theme,
                self.preferences.language == Language::Chinese,
                cx.listener(|this, index: &usize, _, cx| {
                    if let Some(theme) = [Theme::System, Theme::Light, Theme::Dark].get(*index) {
                        this.set_theme(*theme, cx);
                    }
                }),
            ))
            .child(
                Button::new("language")
                    .small()
                    .w(px(112.))
                    .label(self.display_text(current_language.native_name()))
                    .dropdown_caret(true)
                    .accessibility_label(self.text("界面语言", "Interface language"))
                    .dropdown_menu(move |menu, _, _| {
                        Language::ALL.into_iter().fold(menu, |menu, language| {
                            let view = view.clone();
                            menu.item(
                                crate::chrome::choice_item(
                                    language.native_name(),
                                    language == current_language,
                                    dark,
                                )
                                .on_click(move |_, window, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.set_language(language, window, cx)
                                    });
                                }),
                            )
                        })
                    }),
            );
        let downloads = div()
            .flex()
            .items_end()
            .gap_2()
            .child(self.settings_field(
                self.text("并发任务", "Tasks"),
                "max-active",
                &self.active_input,
                muted,
            ))
            .child(self.settings_field(
                self.text("下载线程", "Download threads"),
                "max-connections",
                &self.connections_input,
                muted,
            ))
            .child(self.settings_field(
                self.text("KiB/s · 0 不限", "KiB/s · 0 unlimited"),
                "max-speed",
                &self.speed_input,
                muted,
            ))
            .child(
                Button::new("save-download-settings")
                    .small()
                    .primary()
                    .label(self.display_text(self.text("保存", "Save")))
                    .on_click(cx.listener(|this, _, _, cx| this.save_download_settings(cx))),
            );
        let recovery = Switch::new("auto-resume")
            .small()
            .label(
                self.display_text(self.text("重启后继续下载", "Continue downloads after restart")),
            )
            .tooltip(self.text(
                "自动继续上次运行或排队的任务，手动暂停的任务保持暂停。",
                "Continue previously running or queued tasks. Manually paused tasks stay paused.",
            ))
            .checked(self.preferences.auto_resume)
            .on_click(cx.listener(|this, checked, _, cx| {
                this.pending_auto_resume = *checked;
                this.preferences.auto_resume = *checked;
                this.scheduler
                    .update_options(this.preferences.scheduler_options());
                this.save_preferences(cx);
                cx.notify();
            }));
        let ua_input = self.user_agent_input.downgrade();
        let default_label = self.text("默认（Chrome · Windows）", "Default (Chrome · Windows)");
        let ua_detail = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(recovery)
            .child(self.settings_field(
                "User-Agent",
                "download-user-agent",
                &self.user_agent_input,
                muted,
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("user-agent-presets")
                            .small()
                            .label(self.display_text(self.text("预设", "Presets")))
                            .dropdown_caret(true)
                            .dropdown_menu(move |menu, _, _| {
                                use gpui_kit::component::menu::PopupMenuItem;
                                std::iter::once((default_label, ""))
                                    .chain(USER_AGENT_PRESETS)
                                    .fold(menu, |menu, (label, value)| {
                                        let input = ua_input.clone();
                                        menu.item(PopupMenuItem::new(label).on_click(
                                            move |_, window, cx| {
                                                let _ = input.update(cx, |state, cx| {
                                                    state.set_value(value, window, cx)
                                                });
                                            },
                                        ))
                                    })
                            }),
                    )
                    .child(
                        Button::new("save-user-agent")
                            .small()
                            .primary()
                            .label(self.display_text(self.text("保存", "Save")))
                            .on_click(cx.listener(|this, _, _, cx| this.save_user_agent(cx))),
                    ),
            );
        let motion = div().flex().items_center().child(
            Switch::new("ui-animations")
                .small()
                .checked(self.preferences.ui_animations)
                .label(self.display_text(self.text("界面动效", "Interface motion")))
                .on_click(cx.listener(|this, checked, _, cx| {
                    this.preferences.ui_animations = *checked;
                    cx.set_reduce_motion(!this.preferences.ui_animations);
                    this.save_preferences(cx);
                    cx.notify();
                })),
        );
        let proxy = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                "保存代理后，可用下载窗口的闪电按钮单独启用；默认沿用系统网络设置。",
                "Save a proxy, then use the download window’s lightning button for one task. System network settings apply by default.",
            )))
            .child(self.settings_field(
                self.text("HTTP(S) 代理地址", "HTTP(S) proxy URL"),
                "proxy-url",
                &self.proxy_url_input,
                muted,
            ))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(self.settings_field(
                        self.text("用户名", "Username"),
                        "proxy-user",
                        &self.proxy_user_input,
                        muted,
                    ))
                    .child(self.settings_field(
                        self.text("密码", "Password"),
                        "proxy-password",
                        &self.proxy_password_input,
                        muted,
                    )),
            )
            .child(
                Button::new("proxy-default")
                    .small()
                    .label(if self.pending_proxy_enabled {
                        self.text("新任务默认使用代理：开启", "Proxy for new downloads: On")
                    } else {
                        self.text("新任务默认使用代理：关闭", "Proxy for new downloads: Off")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pending_proxy_enabled = !this.pending_proxy_enabled;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save-proxy-settings")
                    .small()
                    .primary()
                    .label(self.display_text(self.text("保存", "Save")))
                    .on_click(cx.listener(|this, _, _, cx| this.save_download_settings(cx))),
            );
        let extensions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                Button::new("chrome-extension")
                    .small()
                    .icon(IconName::Globe)
                    .label("Chrome")
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url(CHROME_EXTENSION_URL))),
            )
            .child(
                Button::new("edge-extension")
                    .small()
                    .icon(IconName::Globe)
                    .label("Edge")
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url(EDGE_EXTENSION_URL))),
            )
            .child(
                Button::new("extension-guide")
                    .small()
                    .ghost()
                    .icon(IconName::ExternalLink)
                    .label(self.display_text(self.text("手动安装", "Manual install")))
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url(EXTENSION_GUIDE_URL))),
            );
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let extensions = extensions.child(
            div().w_full().flex().flex_col().gap_2()
                .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                    "旧版手动安装的扩展：复制扩展 ID 到下方，授权后即可连接。新版 Release 扩展已自动授权。",
                    "Older unpacked extensions: paste their ID below to allow the connection. New release extensions are already allowed.",
                )))
                .child(div().flex().items_center().gap_2()
                    .child(Input::new(&self.extension_id_input).small().flex_1())
                    .child(Button::new("repair-browser-integration").small()
                        .label(self.display_text(self.text("修复连接", "Repair connection")))
                        .on_click(cx.listener(|this, _, _, cx| this.repair_browser_integration(cx))))),
        );
        let about = card_surface(5, tone)
            .p_3()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(Icon::new(IconName::Info).with_size(px(19.)))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.display_text(self.text("关于与更新", "About and updates"))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(muted))
                    .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(90.))
                    .text_xs()
                    .text_color(rgb(muted))
                    .child(self.update_status()),
            )
            .child(
                Switch::new("automatic-updates")
                    .small()
                    .checked(self.preferences.auto_update)
                    .label(self.display_text(self.text("自动检查", "Auto check")))
                    .tooltip(self.text(
                        "每 12 小时检查一次，点击升级后下载安装。",
                        "Checks every 12 hours. Click upgrade to download and install.",
                    ))
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.preferences.auto_update = *checked;
                        this.save_preferences(cx);
                        if *checked {
                            this.check_for_update(false, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                crate::chrome::icon_button(
                    "update-release-notes",
                    IconName::ExternalLink,
                    self.text("发行说明", "Release notes"),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let url = match &this.update_state {
                        UpdateState::Available(release) => updates::release_url(&release.version),
                        UpdateState::Ready(ready) => updates::release_url(&ready.release.version),
                        _ => format!(
                            "https://github.com/AreChen/NDM-Next/releases/tag/desktop-v{}",
                            env!("CARGO_PKG_VERSION")
                        ),
                    };
                    cx.open_url(&url);
                })),
            )
            .child(self.update_button(cx))
            .when(
                matches!(self.update_state, UpdateState::Downloading),
                |panel| {
                    let ratio = self
                        .update_transfer
                        .as_ref()
                        .map_or(0., |transfer| {
                            transfer.progress.snapshot().downloaded as f32
                                / transfer.expected.max(1) as f32
                        })
                        .clamp(0., 1.);
                    panel.child(
                        div()
                            .h(px(3.))
                            .w_full()
                            .rounded_full()
                            .bg(rgb(tone.rgb(0x28344b, 0xe3e8f2)))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(ratio))
                                    .rounded_full()
                                    .bg(rgb(0x7c68ff)),
                            ),
                    )
                },
            );
        let appearance = self.settings_group(
            (
                0,
                IconName::Sun,
                self.text("外观与语言", "Appearance and language"),
                self.text("主题、语言与界面动效", "Theme, language and motion"),
            ),
            appearance,
            Some(motion.into_any_element()),
            frames[0],
            cx,
        );
        let downloads = self.settings_group(
            (
                1,
                IconName::ArrowDown,
                self.text("下载行为", "Download behavior"),
                self.text(
                    "并发任务、下载线程、限速、恢复及 User-Agent",
                    "Tasks, threads, speed, recovery and User-Agent",
                ),
            ),
            downloads,
            Some(ua_detail.into_any_element()),
            frames[1],
            cx,
        );
        let storage_quick = div()
            .flex()
            .items_center()
            .gap_2()
            .child(Icon::new(IconName::Folder).small())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .truncate()
                    .child(self.preferences.download_dir.to_string_lossy().into_owned()),
            )
            .child(
                Button::new("storage-edit")
                    .small()
                    .label(self.display_text(self.text("管理", "Manage")))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_settings_group(6, window, cx);
                    })),
            );
        let storage = self.settings_group(
            (
                6,
                IconName::Folder,
                self.text("存储与缓存", "Storage and cache"),
                self.text(
                    "下载目录、缓存位置与自动清理",
                    "Download folder, cache and cleanup",
                ),
            ),
            storage_quick,
            Some(self.storage_settings(dark, cx)),
            frames[6],
            cx,
        );
        let proxy_quick = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().text_xs().child(
                if self.preferences.default_proxy().is_some() {
                    self.text(
                        "新任务默认使用已配置代理",
                        "Configured proxy for new downloads",
                    )
                } else {
                    self.text(
                        "默认使用系统网络 · 可按任务启用代理",
                        "System network · optional per-task proxy",
                    )
                },
            ))
            .child(
                Button::new("proxy-edit")
                    .small()
                    .label(self.display_text(self.text("配置", "Configure")))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_settings_group(2, window, cx);
                    })),
            );
        let proxy = self.settings_group(
            (
                2,
                IconName::Globe,
                self.text("代理与网络", "Proxy and network"),
                self.text(
                    "按需启用，独立保存认证信息",
                    "Optional proxy and authentication",
                ),
            ),
            proxy_quick,
            Some(proxy.into_any_element()),
            frames[2],
            cx,
        );
        let mut cards = vec![appearance, downloads, storage, proxy];
        #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
        {
            let startup = Switch::new("startup-toggle")
                .small()
                .label(self.display_text(self.text("开机自启", "Start on sign-in")))
                .checked(self.startup_enabled)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_startup(cx)));
            cards.push(self.settings_group(
                (
                    3,
                    IconName::Settings2,
                    self.text("启动与后台", "Startup and background"),
                    if cfg!(target_os = "linux") {
                        self.text("登录后最小化启动", "Start minimized on sign-in")
                    } else {
                        self.text(
                            "开机自启 · 关闭窗口后留在托盘",
                            "Start on sign-in · close to system tray",
                        )
                    },
                ),
                startup,
                None,
                frames[3],
                cx,
            ));
        }
        cards.push(self.settings_group(
            (
                4,
                IconName::Globe,
                self.text("浏览器扩展", "Browser extension"),
                self.text(
                    "浏览器接管与媒体嗅探",
                    "Download capture and media discovery",
                ),
            ),
            extensions,
            None,
            frames[4],
            cx,
        ));
        let wide =
            two_columns(window.viewport_size().width.as_f32() - self.sidebar.display_width() - 32.);
        let mut panel =
            div()
                .w_full()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .pb_1()
                        .child(
                            div()
                                .size(px(36.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_lg()
                                .bg(rgba(tone.rgba(0x885dff28, 0x885dff16)))
                                .child(Icon::new(IconName::Settings2).with_size(px(22.))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_lg().font_weight(FontWeight::BOLD).child(
                                    self.display_text(self.text("设置中心", "Settings center")),
                                ))
                                .child(div().text_xs().text_color(rgb(muted)).child(
                                    self.display_text(
                                        self.text("个性化你的下载体验", "Make downloads your own"),
                                    ),
                                )),
                        ),
                );
        panel = panel.child(self.ai_panel(frames[plugins::AI_CLIENTS_GROUP], cx));
        let mut cards = cards.into_iter();
        let mut row_index = 0;
        while let Some(first) = cards.next() {
            let second = if wide { cards.next() } else { None };
            // Expanded path/authentication forms get the entire row, rather
            // than squeezing inputs and localized controls into half a card.
            let stack = row_index == 1 && (frames[6].is_mounted() || frames[2].is_mounted());
            panel = panel.child(settings_row(first, second, stack));
            row_index += 1;
        }
        panel.child(about)
    }
}

fn toggle_group(open: &mut HashSet<usize>, index: usize) -> bool {
    if open.insert(index) {
        true
    } else {
        open.remove(&index);
        false
    }
}

fn disclosure_frame(index: usize, open: bool, window: &mut Window, cx: &mut App) -> Frame {
    let state = window.use_keyed_state(("settings-disclosure", index), cx, |_, _| {
        MotionState::default()
    });
    state.update(cx, |state, cx| {
        state.frame(
            Motion::new(Target::default().opacity(1.))
                .initial(Target::default().opacity(0.))
                .exit(Target::default().opacity(0.))
                .present(open)
                .transition(crate::motion::tween(220)),
            window,
            cx,
        )
    })
}

pub(super) fn settings_detail(
    index: usize,
    frame: Frame,
    open: bool,
    detail: AnyElement,
) -> AnyElement {
    // Kit provides measured layout/clipping only; gpui-motion-kit owns timing
    // and presence. Keep the outgoing form mounted until its exit finishes.
    let body = div()
        .w_full()
        .when(frame.is_mounted(), |body| {
            body.pt_3().opacity(frame.target().alpha()).child(detail)
        })
        .when(!open, |body| {
            body.capture_any_mouse_down(|_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
        });
    gpui_kit::base::MotionReveal::new(
        ("settings-reveal", index),
        frame.target().alpha(),
        body.into_any_element(),
    )
    .into_any_element()
}

fn theme_switch(
    theme: Theme,
    zh: bool,
    on_change: impl Fn(&usize, &mut Window, &mut App) + 'static,
) -> SegmentedControl {
    SegmentedControl {
        id: "theme-switch".into(),
        segments: [
            ("theme-system", if zh { "跟随系统" } else { "System" }),
            ("theme-light", if zh { "亮色" } else { "Light" }),
            ("theme-dark", if zh { "暗色" } else { "Dark" }),
        ]
        .into_iter()
        .map(|(id, label)| (id.into(), label.into()))
        .collect(),
        selected: Some(match theme {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        }),
        accessibility_label: Some(if zh { "主题" } else { "Theme" }.into()),
        width: None,
        disabled: false,
        on_change: std::rc::Rc::new(on_change),
    }
}

// Each half needs room for localized labels and real input controls. Sidebar
// resizing changes the available content width without changing saved settings.
fn two_columns(available: f32) -> bool {
    available >= 760.
}

fn settings_row(first: AnyElement, second: Option<AnyElement>, stack: bool) -> Div {
    div()
        .w_full()
        .flex_shrink_0()
        .flex()
        .items_stretch()
        .gap_3()
        .when(stack, |row| row.flex_col())
        .child(first)
        .children(second)
}

fn settings_card(
    index: usize,
    header: impl IntoElement,
    quick: impl IntoElement,
    detail: Option<AnyElement>,
    tone: impl Into<crate::motion::ThemeTone>,
) -> Stateful<Div> {
    let tone = tone.into();
    card_surface(index, tone)
        .p_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(header)
        .child(
            div()
                .border_t_1()
                .border_color(rgb(tone.rgb(0x334156, 0xe1e6f0)))
                .pt_3()
                .flex()
                .flex_col()
                .child(quick)
                .children(detail),
        )
}

pub(super) fn card_surface(
    index: usize,
    tone: impl Into<crate::motion::ThemeTone>,
) -> Stateful<Div> {
    let tone = tone.into();
    div()
        .id(("settings-card", index))
        .debug_selector(move || format!("settings-card-{index}"))
        .flex_1()
        .min_w_0()
        .flex_shrink_0()
        .rounded_lg()
        .border_1()
        .border_color(rgb(tone.rgb(0x354260, 0xdce1f0)))
        .bg(linear_gradient(
            110.,
            linear_color_stop(rgb(tone.rgb(0x1d2039, 0xfaf8ff)), 0.),
            linear_color_stop(rgb(tone.rgb(0x142b35, 0xf2f9fc)), 1.),
        ))
}

#[cfg(test)]
mod tests {
    use super::{settings_card, settings_row, theme_switch, two_columns};
    use crate::preferences::Theme;
    use gpui::component::{
        Sizable,
        button::Button,
        input::{Input, InputState},
    };
    use gpui::{
        Context, Entity, IntoElement, Modifiers, Render, Window, div, prelude::*, px, size,
    };
    use gpui_kit as gpui;

    struct Cards {
        input: Entity<InputState>,
        open: bool,
        dark: bool,
        zh: bool,
    }

    struct Disclosure {
        index: usize,
        open: HashSet<usize>,
        input: Entity<InputState>,
    }

    use std::collections::HashSet;

    impl Disclosure {
        fn toggle(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
            super::toggle_group(&mut self.open, self.index);
            cx.notify();
        }
    }

    impl Render for Disclosure {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let open = self.open.contains(&self.index);
            let frame = super::disclosure_frame(self.index, open, window, cx);
            div().size_full().p_4().child(
                settings_card(
                    self.index,
                    Button::new("section-toggle")
                        .small()
                        .debug_selector(|| "section-toggle".into())
                        .label("More settings")
                        .on_click(cx.listener(Self::toggle)),
                    Button::new("quick-toggle")
                        .small()
                        .debug_selector(|| "quick-toggle".into())
                        .label(if self.index == 6 {
                            "Manage"
                        } else {
                            "Configure"
                        })
                        .on_click(cx.listener(Self::toggle)),
                    Some(super::settings_detail(
                        self.index,
                        frame,
                        open,
                        div()
                            .h(px(128.))
                            .w_full()
                            .child(
                                div()
                                    .debug_selector(|| "disclosed-input".into())
                                    .child(Input::new(&self.input).small()),
                            )
                            .into_any_element(),
                    )),
                    true,
                )
                .w(px(400.)),
            )
        }
    }

    #[gpui::test]
    fn manage_and_configure_reverse_measured_reveals_without_losing_input(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::time::Duration;
        cx.update(gpui::init);
        for (index, card_id) in [(6, "settings-card-6"), (2, "settings-card-2")] {
            let (view, cx) = cx.add_window_view(move |window, cx| Disclosure {
                index,
                open: HashSet::new(),
                input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value("preserved value", window, cx);
                    input
                }),
            });
            cx.simulate_resize(size(px(500.), px(420.)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let collapsed = cx.debug_bounds(card_id).unwrap().size.height;
            assert!(cx.debug_bounds("disclosed-input").is_none());
            for button in ["quick-toggle", "section-toggle"] {
                let point = cx.debug_bounds(button).unwrap().center();
                cx.simulate_click(point, Modifiers::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.background_executor
                    .advance_clock(Duration::from_millis(90));
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let halfway = cx.debug_bounds(card_id).unwrap().size.height;
                cx.background_executor
                    .advance_clock(Duration::from_millis(160));
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let expanded = cx.debug_bounds(card_id).unwrap().size.height;
                assert!(
                    collapsed < halfway && halfway < expanded,
                    "{index}: expand {collapsed:?}/{halfway:?}/{expanded:?}"
                );
                assert!(cx.debug_bounds("disclosed-input").is_some());
                let point = cx.debug_bounds(button).unwrap().center();
                cx.simulate_click(point, Modifiers::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.background_executor
                    .advance_clock(Duration::from_millis(90));
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let halfway = cx.debug_bounds(card_id).unwrap().size.height;
                assert!(
                    collapsed < halfway && halfway < expanded,
                    "{index}: collapse"
                );
                cx.background_executor
                    .advance_clock(Duration::from_millis(160));
                cx.update(|window, cx| window.draw(cx).clear(cx));
                assert_eq!(cx.debug_bounds(card_id).unwrap().size.height, collapsed);
                assert!(cx.debug_bounds("disclosed-input").is_none());
                cx.update(|_, cx| {
                    assert_eq!(view.read(cx).input.read(cx).value(), "preserved value")
                });
            }
            let quick = cx.debug_bounds("quick-toggle").unwrap().center();
            cx.simulate_click(quick, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.background_executor
                .advance_clock(Duration::from_millis(60));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let interrupted = cx.debug_bounds(card_id).unwrap().size.height;
            let arrow = cx.debug_bounds("section-toggle").unwrap().center();
            cx.simulate_click(arrow, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!((cx.debug_bounds(card_id).unwrap().size.height - interrupted).abs() <= px(1.));
            cx.background_executor
                .advance_clock(Duration::from_millis(260));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(cx.debug_bounds(card_id).unwrap().size.height, collapsed);

            cx.update(|_, cx| cx.set_reduce_motion(true));
            cx.simulate_click(quick, Modifiers::default());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds(card_id).unwrap().size.height > collapsed);
            cx.simulate_click(quick, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(cx.debug_bounds(card_id).unwrap().size.height, collapsed);
            assert!(cx.debug_bounds("disclosed-input").is_none());
            cx.update(|_, cx| cx.set_reduce_motion(false));
        }
    }

    #[gpui::test]
    fn theme_segments_fit_beside_language_selector_and_switch_all_three_modes(
        cx: &mut gpui::TestAppContext,
    ) {
        struct Appearance {
            theme: Theme,
            zh: bool,
        }
        impl Render for Appearance {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .p_4()
                    .flex()
                    .items_start()
                    .child(settings_card(
                        0,
                        div().child("Appearance"),
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(theme_switch(
                                self.theme,
                                self.zh,
                                cx.listener(|this, index: &usize, _, cx| {
                                    this.theme = [Theme::System, Theme::Light, Theme::Dark][*index];
                                    cx.notify();
                                }),
                            ))
                            .child(
                                Button::new("language")
                                    .small()
                                    .w(px(112.))
                                    .dropdown_caret(true)
                                    .label(if self.zh { "简体中文" } else { "English" })
                                    .debug_selector(|| "language".into()),
                            ),
                        None,
                        true,
                    ))
            }
        }
        cx.update(gpui::init);
        let (view, cx) = cx.add_window_view(|_, _| Appearance {
            theme: Theme::System,
            zh: true,
        });
        for width in [380., 548.] {
            cx.simulate_resize(size(px(width), px(240.)));
            for zh in [true, false] {
                cx.update(|window, cx| {
                    view.update(cx, |this, cx| {
                        this.zh = zh;
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let card = cx.debug_bounds("settings-card-0").unwrap();
                let language = cx.debug_bounds("language").unwrap();
                assert!(language.right() <= card.right() - px(12.));
                let system = cx.debug_bounds("theme-system").unwrap();
                let light = cx.debug_bounds("theme-light").unwrap();
                let dark = cx.debug_bounds("theme-dark").unwrap();
                assert!((system.size.width - light.size.width).abs() <= px(1.));
                assert!((light.size.width - dark.size.width).abs() <= px(1.));
                assert!(
                    (dark.right() + px(3.) - (language.left() - px(8.))).abs() <= px(1.),
                    "No unused tail inside theme selector"
                );
                for (id, mode) in [
                    ("theme-system", Theme::System),
                    ("theme-light", Theme::Light),
                    ("theme-dark", Theme::Dark),
                ] {
                    let segment = cx.debug_bounds(id).unwrap();
                    assert!(segment.left() >= card.left() + px(12.));
                    assert!(segment.right() <= language.left() - px(8.));
                    assert!(segment.top() < language.bottom() && segment.bottom() > language.top());
                    cx.simulate_click(segment.center(), Modifiers::default());
                    cx.update(|window, cx| {
                        assert_eq!(view.read(cx).theme, mode);
                        window.draw(cx).clear(cx);
                    });
                }
            }
        }
    }

    impl Render for Cards {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let first = settings_card(
                0,
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().truncate().child(if self.zh {
                        "存储与缓存"
                    } else {
                        "Storage and cache"
                    }))
                    .child(
                        Button::new("expand")
                            .small()
                            .debug_selector(|| "expand".into())
                            .label(if self.open { "−" } else { "+" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open = !this.open;
                                cx.notify();
                            })),
                    ),
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(Input::new(&self.input).small().flex_1().min_w_0())
                    .child(
                        Button::new("choose")
                            .small()
                            .debug_selector(|| "choose".into())
                            .label(if self.zh { "选择" } else { "Choose folder" }),
                    ),
                self.open
                    .then(|| div().h(px(100.)).child("Cache settings").into_any_element()),
                self.dark,
            )
            .into_any_element();
            let second = settings_card(
                1,
                div().child(if self.zh {
                    "代理与网络"
                } else {
                    "Proxy and network"
                }),
                Button::new("configure")
                    .small()
                    .debug_selector(|| "configure".into())
                    .label(if self.zh { "配置" } else { "Configure proxy" }),
                None,
                self.dark,
            )
            .into_any_element();
            div()
                .size_full()
                .p_4()
                .flex()
                .flex_col()
                .child(settings_row(
                    first,
                    Some(second),
                    self.open || !two_columns(window.viewport_size().width.as_f32() - 32.),
                ))
        }
    }

    #[gpui::test]
    fn settings_cards_fit_localized_controls_and_reflow_without_losing_input(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui::init);
        let (view, cx) = cx.add_window_view(|window, cx| Cards {
            input: cx.new(|cx| InputState::new(window, cx).default_value("D:/Downloads/下载")),
            open: false,
            dark: true,
            zh: true,
        });
        for width in [548., 788., 1288.] {
            cx.simulate_resize(size(px(width), px(500.)));
            for dark in [true, false] {
                for zh in [true, false] {
                    cx.update(|window, cx| {
                        view.update(cx, |this, cx| {
                            this.dark = dark;
                            this.zh = zh;
                            cx.notify();
                        });
                        window.draw(cx).clear(cx);
                    });
                    for open in [false, true] {
                        cx.update(|window, cx| window.draw(cx).clear(cx));
                        let first = cx.debug_bounds("settings-card-0").unwrap();
                        let second = cx.debug_bounds("settings-card-1").unwrap();
                        if open || !two_columns(width - 32.) {
                            assert!(second.origin.y >= first.bottom() + px(12.));
                        } else {
                            assert_eq!(first.origin.y, second.origin.y);
                            assert!(second.origin.x >= first.right() + px(12.));
                        }
                        for (id, card) in
                            [("choose", first), ("expand", first), ("configure", second)]
                        {
                            let button = cx.debug_bounds(id).unwrap();
                            assert!(button.origin.x >= card.origin.x + px(12.), "{id}");
                            assert!(button.right() <= card.right() - px(12.), "{id}: {button:?}");
                            assert!(button.bottom() <= card.bottom() - px(12.), "{id}");
                        }
                        let toggle = cx.debug_bounds("expand").unwrap();
                        cx.simulate_click(toggle.center(), Modifiers::default());
                        cx.update(|_, cx| assert_eq!(view.read(cx).open, !open));
                    }
                    cx.update(|_, cx| {
                        assert_eq!(
                            view.read(cx).input.read(cx).value().as_str(),
                            "D:/Downloads/下载"
                        );
                    });
                }
            }
        }
    }
}
