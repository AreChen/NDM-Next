use super::*;
use gpui_kit::component::Selectable;

const CHROME_EXTENSION_URL: &str =
    "https://chromewebstore.google.com/detail/ndm-next/caegebfdojcdlkkiaiokeoadefdooamd";
const EDGE_EXTENSION_URL: &str =
    "https://microsoftedge.microsoft.com/addons/detail/ndm-next/hiblommiheggidolemmnnlocmocebckp";
const EXTENSION_GUIDE_URL: &str =
    "https://github.com/AreChen/NDM-Next/blob/main/docs/extension-install.md";

impl Desktop {
    fn settings_group(
        &self,
        group: (usize, IconName, &'static str, &'static str),
        body: impl IntoElement,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (index, icon, title, summary) = group;
        let open = self.settings_open.contains(&index);
        div()
            .w_full()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if dark { 0x28344b } else { 0xe3e8f2 }))
            .bg(rgb(if dark { 0x141d2d } else { 0xffffff }))
            .child(
                Button::new(("settings-section", index))
                    .w_full()
                    .ghost()
                    .h(px(48.))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(Icon::new(icon).with_size(px(16.)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(if dark { 0x95a5c0 } else { 0x71809a }))
                                            .truncate()
                                            .child(summary),
                                    ),
                            )
                            .child(
                                Icon::new(if open {
                                    IconName::ChevronUp
                                } else {
                                    IconName::ChevronDown
                                })
                                .small(),
                            ),
                    )
                    .accessibility_label(title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.settings_open.insert(index) {
                            this.settings_open.remove(&index);
                        }
                        if index == 6 && this.settings_open.contains(&index) {
                            this.maintain_storage(false, false, cx);
                        }
                        cx.notify();
                    })),
            )
            .when(open, |group| {
                group.child(div().p_3().flex().flex_col().gap_2().child(body))
            })
    }

    pub(super) fn settings_panel(&self, dark: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = if dark { 0x95a5c0 } else { 0x71809a };
        let appearance = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("theme-system")
                    .small()
                    .label(self.text("跟随系统", "System"))
                    .selected(self.preferences.theme == Theme::System)
                    .on_click(cx.listener(|this, _, _, cx| this.set_theme(Theme::System, cx))),
            )
            .child(
                Button::new("theme-light")
                    .small()
                    .label(self.text("亮色", "Light"))
                    .selected(self.preferences.theme == Theme::Light)
                    .on_click(cx.listener(|this, _, _, cx| this.set_theme(Theme::Light, cx))),
            )
            .child(
                Button::new("theme-dark")
                    .small()
                    .label(self.text("暗色", "Dark"))
                    .selected(self.preferences.theme == Theme::Dark)
                    .on_click(cx.listener(|this, _, _, cx| this.set_theme(Theme::Dark, cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("language")
                    .small()
                    .label(if self.preferences.language == Language::Chinese {
                        "English"
                    } else {
                        "中文"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clear_notice();
                        this.preferences.language =
                            if this.preferences.language == Language::Chinese {
                                Language::English
                            } else {
                                Language::Chinese
                            };
                        #[cfg(any(windows, target_os = "macos"))]
                        if let Some(tray) = &this.tray {
                            tray.set_language(this.preferences.language);
                        }
                        for (_, progress) in this.progress_windows.values() {
                            if let Some(progress) = progress.upgrade() {
                                progress.update(cx, |view, cx| {
                                    view.language = this.preferences.language;
                                    cx.notify();
                                });
                            }
                        }
                        this.save_preferences(cx);
                        cx.notify();
                    })),
            );
        let downloads = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(self.settings_field(
                        self.text("同时下载", "Concurrent downloads"),
                        "max-active",
                        &self.active_input,
                        muted,
                    ))
                    .child(self.settings_field(
                        self.text("HTTP(S) 连接数", "HTTP(S) connections"),
                        "max-connections",
                        &self.connections_input,
                        muted,
                    ))
                    .child(self.settings_field(
                        self.text("限速 KiB/s · 0 不限", "Limit KiB/s · 0 unlimited"),
                        "max-speed",
                        &self.speed_input,
                        muted,
                    )),
            )
            .child(
                Button::new("auto-resume")
                    .small()
                    .label(if self.pending_auto_resume {
                        self.text("启动时恢复下载：开启", "Resume on startup: On")
                    } else {
                        self.text("启动时恢复下载：关闭", "Resume on startup: Off")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pending_auto_resume = !this.pending_auto_resume;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save-download-settings")
                    .small()
                    .primary()
                    .label(self.text("保存", "Save"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_download_settings(cx))),
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
                    .label(self.text("保存", "Save"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_download_settings(cx))),
            );
        let extensions = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                "安装扩展，让浏览器下载自动交给 NDM Next。商店审核期间可手动安装。",
                "Capture browser downloads with the extension. Manual installation is available during store review.",
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("chrome-extension")
                            .small()
                            .icon(IconName::Globe)
                            .label("Chrome / Brave")
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(CHROME_EXTENSION_URL))),
                    )
                    .child(
                        Button::new("edge-extension")
                            .small()
                            .icon(IconName::Globe)
                            .label("Microsoft Edge")
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(EDGE_EXTENSION_URL))),
                    )
                    .child(
                        Button::new("extension-guide")
                            .small()
                            .ghost()
                            .icon(IconName::ExternalLink)
                            .label(self.text("手动安装", "Manual install"))
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(EXTENSION_GUIDE_URL))),
                    ),
            );
        #[cfg(target_os = "macos")]
        let extensions = extensions.child(
            div().flex().flex_col().gap_2()
                .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                    "旧版手动安装的扩展：复制扩展 ID 到下方，授权后即可连接。新版 Release 扩展已自动授权。",
                    "Older unpacked extensions: paste their ID below to allow the connection. New release extensions are already allowed.",
                )))
                .child(div().flex().items_center().gap_2()
                    .child(Input::new(&self.extension_id_input).small().flex_1())
                    .child(Button::new("repair-browser-integration").small()
                        .label(self.text("修复连接", "Repair connection"))
                        .on_click(cx.listener(|this, _, _, cx| this.repair_browser_integration(cx))))),
        );
        let about = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_sm()
                    .child(crate::branding::logo(px(28.)))
                    .child(format!("NDM Next {}", env!("CARGO_PKG_VERSION")))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(muted))
                            .child(self.update_status()),
                    )
                    .child(self.update_button(cx))
                    .child(Button::new("update-release-notes").small().ghost().icon(IconName::ExternalLink)
                        .tooltip(self.text("发行说明", "Release notes"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            let url = match &this.update_state {
                                UpdateState::Available(release) => updates::release_url(&release.version),
                                UpdateState::Ready(ready) => updates::release_url(&ready.release.version),
                                _ => format!("https://github.com/AreChen/NDM-Next/releases/tag/desktop-v{}", env!("CARGO_PKG_VERSION")),
                            };
                            cx.open_url(&url);
                        }))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("automatic-updates")
                            .small()
                            .selected(self.preferences.auto_update)
                            .label(if self.preferences.auto_update {
                                self.text("自动检查：开启", "Auto check: On")
                            } else {
                                self.text("自动检查：关闭", "Auto check: Off")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.auto_update = !this.preferences.auto_update;
                                this.save_preferences(cx);
                                if this.preferences.auto_update {
                                    this.check_for_update(false, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().text_xs().text_color(rgb(muted)).child(self.text(
                        "每 12 小时检查 · 点击升级后下载、安装",
                        "Checks every 12 hours · click upgrade to download and install",
                    ))),
            );
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.settings_group(
                (
                    0,
                    IconName::Sun,
                    self.text("外观与语言", "Appearance and language"),
                    self.text(
                        "亮色、暗色、跟随系统 · 中文 / English",
                        "Light, dark, system · 中文 / English",
                    ),
                ),
                appearance,
                dark,
                cx,
            ))
            .child(self.settings_group(
                (
                    1,
                    IconName::ArrowDown,
                    self.text("下载行为", "Download behavior"),
                    self.text(
                        "并发、连接数、限速与恢复",
                        "Concurrency, connections, rate limit and recovery",
                    ),
                ),
                downloads,
                dark,
                cx,
            ))
            .child(self.settings_group(
                (
                    6,
                    IconName::Folder,
                    self.text("存储与缓存", "Storage and cache"),
                    self.text(
                        "下载目录、缓存位置与自动清理",
                        "Download folder, cache location and cleanup",
                    ),
                ),
                self.storage_settings(dark, cx),
                dark,
                cx,
            ))
            .child(self.settings_group(
                (
                    2,
                    IconName::Globe,
                    self.text("代理与网络", "Proxy and network"),
                    self.text("可选代理及认证信息", "Optional proxy and authentication"),
                ),
                proxy,
                dark,
                cx,
            ));
        #[cfg(any(windows, target_os = "macos"))]
        {
            let startup = Button::new("startup-toggle")
                .small()
                .label(if self.startup_enabled {
                    self.text("开机自启：开启", "Start on sign-in: On")
                } else {
                    self.text("开机自启：关闭", "Start on sign-in: Off")
                })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_startup(cx)));
            panel = panel.child(self.settings_group(
                (
                    3,
                    IconName::Settings2,
                    self.text("启动与后台", "Startup and background"),
                    self.text(
                        "开机自启 · 关闭窗口后留在托盘",
                        "Start on sign-in · close to system tray",
                    ),
                ),
                startup,
                dark,
                cx,
            ));
        }
        panel = panel.child(self.settings_group(
            (
                4,
                IconName::Globe,
                self.text("浏览器扩展", "Browser extension"),
                "Chrome / Brave · Microsoft Edge",
            ),
            extensions,
            dark,
            cx,
        ));
        panel.child(self.settings_group(
            (
                5,
                IconName::Info,
                self.text("关于与更新", "About and updates"),
                self.text("版本信息与更新检查", "Version and update checks"),
            ),
            about,
            dark,
            cx,
        ))
    }
}
