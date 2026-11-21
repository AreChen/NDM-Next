use super::*;

pub(super) struct Editor {
    pub(super) fields: Vec<Field>,
    plugin: InstalledPlugin,
}

impl Desktop {
    pub(super) fn open_plugin_defaults(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(plugin) = self
            .integration
            .market
            .installed
            .iter()
            .find(|plugin| plugin.manifest.id == id && !plugin.manifest.options.is_empty())
            .cloned()
        {
            let fields = option_fields(
                plugin.manifest.options.clone(),
                Some(&plugin.option_defaults),
                window,
                cx,
            );
            self.integration.defaults = Some(Editor { fields, plugin });
            self.clear_notice();
            self.page_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(in crate::view) fn plugin_defaults_open(&self) -> bool {
        self.integration.defaults.is_some()
    }

    fn close_plugin_defaults(&mut self, cx: &mut Context<Self>) {
        if self.integration.defaults.take().is_some() {
            self.clear_notice();
            cx.notify();
        }
    }

    fn save_plugin_defaults(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.integration.defaults else {
            return;
        };
        if self.integration.market.busy || matches!(self.update_state, UpdateState::Installing) {
            return;
        }
        let values = match self.read_plugin_options(&editor.fields, cx) {
            Ok(values) => values,
            Err(message) => {
                self.set_notice(message.into(), cx);
                return;
            }
        };
        let plugin = editor.plugin.clone();
        if values
            .iter()
            .any(|(key, value)| !plugin.manifest.accepts_option(key, value))
        {
            self.set_notice(
                self.text("请检查插件选项。", "Check the plugin options.")
                    .into(),
                cx,
            );
            return;
        }
        let registry = self.registry();
        let runtime = self.runtime.clone();
        self.integration.market.busy = true;
        cx.spawn(async move |view, cx| {
            let result = runtime
                .spawn_blocking(move || registry.save_option_defaults(&plugin, values))
                .await;
            let _ = view.update(cx, |this, cx| {
                this.integration.market.busy = false;
                match result {
                    Ok(Ok(saved)) => {
                        if let Some(plugin) = this
                            .integration
                            .market
                            .installed
                            .iter_mut()
                            .find(|plugin| plugin.directory == saved.directory)
                        {
                            plugin.option_defaults = saved.option_defaults;
                        }
                        this.integration.defaults = None;
                        this.integration.draft = None;
                        this.set_notice(
                            this.text("插件默认设置已保存。", "Plugin defaults saved.")
                                .into(),
                            cx,
                        );
                    }
                    _ => {
                        this.set_notice(
                            this.text(
                                "无法保存插件设置。请刷新市场后重试。",
                                "Could not save plugin defaults. Refresh the market and try again.",
                            )
                            .into(),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn plugin_defaults_panel(
        &self,
        editor: &Editor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tone = crate::motion::ThemeTone::current(cx);
        let name = editor
            .plugin
            .manifest
            .name
            .for_language(self.preferences.language == Language::Chinese)
            .to_owned();
        let busy = self.integration.market.busy;
        let blocked = busy || matches!(self.update_state, UpdateState::Installing);
        let controls = self.plugin_option_controls(&editor.fields, OptionForm::Defaults, cx);
        div()
            .id("plugin-defaults")
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .items_start()
                    .gap_2()
                    .child(
                        Button::new("back-to-market")
                            .small()
                            .ghost()
                            .icon(IconName::ArrowLeft)
                            .label(self.text("返回市场", "Back to market"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_plugin_defaults(cx);
                                this.page_focus.focus(window, cx);
                            })),
                    )
                    .child(div().w_full().truncate().text_xl().font_weight(FontWeight::BOLD).child(name)),
            )
            .child(
                div()
                    .id("plugin-defaults-options")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        crate::view::settings::card_surface(0, tone)
                            .flex()
                            .flex_col()
                            .gap_3()
                            .p_3()
                            .child(
                                div().font_weight(FontWeight::SEMIBOLD)
                                    .child(self.text("默认下载选项", "Download defaults")),
                            )
                            .child(
                                div().text_xs().text_color(rgb(tone.rgb(0xa9b7d2, 0x586986)))
                                    .child(self.text(
                                        "用于此插件的新任务。添加下载时可以单独修改。",
                                        "Used for new tasks with this plugin. You can edit options when adding a download.",
                                    )),
                            )
                            .child(controls),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .flex_shrink_0()
                    .gap_2()
                    .child(
                        Button::new("reset-plugin-defaults")
                            .small()
                            .ghost()
                            .disabled(blocked)
                            .label(self.text("恢复默认", "Restore defaults"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(editor) = &mut this.integration.defaults {
                                    editor.fields = option_fields(
                                        editor.plugin.manifest.options.clone(),
                                        None,
                                        window,
                                        cx,
                                    );
                                    this.clear_notice();
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("save-plugin-defaults")
                            .small()
                            .primary()
                            .disabled(blocked)
                            .loading(busy)
                            .label(self.text("保存设置", "Save settings"))
                            .on_click(cx.listener(|this, _, _, cx| this.save_plugin_defaults(cx))),
                    ),
            )
            .into_any_element()
    }
}
