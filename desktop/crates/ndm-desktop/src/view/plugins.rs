use super::*;
use crate::motion::SegmentedControl;
use gpui_kit::component::Disableable;
use ndm_core::plugins::{
    Catalog, CatalogPlugin, InstalledPlugin, OptionControl, OptionField, Panel, PluginRegistry,
    built_in_source, parse_source,
};
use ndm_host::harness::{CodexCli, Harness, Status, Target};
use serde_json::{Map, Value, json};

mod defaults;
mod market;
pub(super) use market::market_search_placeholder;

#[derive(Default)]
pub(super) struct Integration {
    market: market::Market,
    defaults: Option<defaults::Editor>,
    ai_enabled: bool,
    codex: Option<Status>,
    codex_busy: bool,
    draft: Option<Draft>,
    draft_work: Option<gpui_kit::Task<()>>,
}

struct Draft {
    source: String,
    candidates: Vec<InstalledPlugin>,
    selected: Option<String>,
    native: bool,
    manual: bool,
    ready: bool,
    panel_error: bool,
    fields: Vec<Field>,
}

struct Field {
    definition: OptionField,
    value: Option<Value>,
    input: Option<Entity<InputState>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OptionForm {
    Download,
    Defaults,
}

enum PackageAction {
    Install(Box<CatalogPlugin>),
    Import(PathBuf),
    Remove(String),
}

#[derive(Clone, Copy)]
enum CodexAction {
    Inspect,
    Install,
    Remove,
}

impl Integration {
    pub(super) fn new(store: &Store) -> Self {
        Self {
            ai_enabled: store.ai_enabled().unwrap_or(false),
            ..Self::default()
        }
    }
}

impl Desktop {
    fn registry(&self) -> PluginRegistry {
        PluginRegistry::new(self.store.profile_directory())
    }

    pub(super) fn open_plugin_market(&mut self, cx: &mut Context<Self>) {
        if self.integration.market.catalog.is_none() && !self.integration.market.busy {
            self.refresh_plugin_market(cx);
        }
    }

    fn refresh_plugin_market(&mut self, cx: &mut Context<Self>) {
        let market = &mut self.integration.market;
        if market.busy {
            return;
        }
        market.busy = true;
        market.catalog_error = false;
        let registry = self.registry();
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let results = runtime
                .spawn(async move {
                    let installed = registry.list();
                    (installed, Catalog::fetch().await)
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                let market = &mut this.integration.market;
                market.busy = false;
                match results {
                    Ok((installed, catalog)) => {
                        if let Ok(installed) = installed {
                            market.installed = installed;
                        }
                        match catalog {
                            Ok(catalog) => market.catalog = Some(catalog),
                            Err(_) => market.catalog_error = true,
                        }
                    }
                    Err(_) => market.catalog_error = true,
                }
                market.rebuild();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn package_action(&mut self, action: PackageAction, cx: &mut Context<Self>) {
        if self.integration.market.busy || matches!(self.update_state, UpdateState::Installing) {
            return;
        }
        self.integration.market.busy = true;
        let registry = self.registry();
        let scheduler = self.scheduler.clone();
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let result = runtime.spawn(async move {
                match action {
                    PackageAction::Install(plugin) => registry.download_and_install(&plugin).await.map(|_| ())?,
                    PackageAction::Import(path) => { registry.import(&path)?; }
                    PackageAction::Remove(id) => scheduler.uninstall_plugin(&id).map_err(std::io::Error::other)?,
                }
                registry.list()
            }).await;
            let _ = view.update(cx, |this, cx| {
                this.integration.market.busy = false;
                match result {
                    Ok(Ok(installed)) => {
                        this.integration.market.installed = installed;
                        this.integration.market.rebuild();
                        this.integration.draft = None;
                        this.scheduler.notify();
                        this.set_notice(this.text("插件已更新。", "Plugin changes saved.").into(), cx);
                    }
                    _ => this.set_notice(this.text(
                        "无法完成插件操作。请检查安装包，或先暂停并移除该插件的排队任务。",
                        "Could not change the plugin. Check the package or clear its active and queued tasks.",
                    ).into(), cx),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn pick_plugin_package(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(self.text("选择插件 ZIP", "Choose plugin ZIP").into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = view.update_in(cx, |this, _, cx| {
                    this.package_action(PackageAction::Import(path), cx)
                });
            }
        })
        .detach();
    }

    pub(super) fn pick_download_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                self.text("选择下载来源文件", "Choose a download source file")
                    .into(),
            ),
        });
        cx.spawn_in(window, async move |view, cx| {
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.into_iter().next()
                && let Ok(url) = reqwest::Url::from_file_path(path)
            {
                let _ = view.update_in(cx, |this, window, cx| {
                    this.url_input
                        .update(cx, |input, cx| input.set_value(url.as_str(), window, cx));
                });
            }
        })
        .detach();
    }

    pub(super) fn update_draft(
        &mut self,
        source: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .integration
            .draft
            .as_ref()
            .is_some_and(|draft| draft.source == source)
        {
            return;
        }
        self.refresh_draft(source, None, window, cx);
    }

    fn refresh_draft(
        &mut self,
        source: String,
        selection: Option<Option<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.integration.draft_work = None;
        let Ok(parsed) = parse_source(&source) else {
            self.integration.draft = None;
            cx.notify();
            return;
        };
        let native = built_in_source(&parsed);
        let registry = self.registry();
        let runtime = self.runtime.clone();
        let chinese = self.preferences.language == Language::Chinese;
        let manual = selection.is_some();
        self.integration.draft = Some(Draft {
            source: source.clone(),
            candidates: Vec::new(),
            selected: None,
            native,
            manual,
            ready: false,
            panel_error: false,
            fields: Vec::new(),
        });
        self.integration.draft_work = Some(cx.spawn_in(window, async move |view, cx| {
            if !manual {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
            }
            let result = runtime
                .spawn_blocking(move || {
                    let candidates = registry.candidates(&parsed).unwrap_or_default();
                    let selected = selection.unwrap_or_else(|| {
                        candidates
                            .first()
                            .map(|candidate| candidate.manifest.id.clone())
                    });
                    let (fields, failed) = match selected
                        .as_ref()
                        .and_then(|id| candidates.iter().find(|plugin| &plugin.manifest.id == id))
                    {
                        None => (Vec::new(), false),
                        Some(plugin) => match panel_fields(plugin, parsed.scheme(), chinese) {
                            Ok(fields) => (fields, false),
                            Err(_) => (plugin.manifest.options.clone(), true),
                        },
                    };
                    (candidates, selected, fields, failed)
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                if this
                    .integration
                    .draft
                    .as_ref()
                    .is_none_or(|draft| draft.source != source)
                {
                    return;
                }
                let (candidates, selected, fields, panel_error) = result.unwrap_or_default();
                let saved_defaults = candidates
                    .iter()
                    .find(|plugin| selected.as_ref() == Some(&plugin.manifest.id))
                    .map(|plugin| &plugin.option_defaults);
                let fields = option_fields(fields, saved_defaults, window, cx);
                this.integration.draft = Some(Draft {
                    source,
                    candidates,
                    selected,
                    native,
                    manual,
                    ready: true,
                    panel_error,
                    fields,
                });
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn plugin_task_options(
        &self,
        source: &str,
        cx: &App,
    ) -> Result<Option<TaskOptions>, &'static str> {
        let Some(draft) = self
            .integration
            .draft
            .as_ref()
            .filter(|draft| draft.source == source && draft.ready)
        else {
            return Err(self.text("正在匹配下载引擎。", "Looking up download engines."));
        };
        let Some(id) = draft.selected.as_ref() else {
            return Ok(None);
        };
        let plugin = draft
            .candidates
            .iter()
            .find(|plugin| &plugin.manifest.id == id)
            .ok_or(self.text("插件已不可用。", "Plugin is unavailable."))?;
        let mut values = plugin.option_defaults.clone();
        for field in &draft.fields {
            values.remove(&field.definition.key);
        }
        values.extend(self.read_plugin_options(&draft.fields, cx)?);
        let values = plugin
            .manifest
            .complete_options(values)
            .map_err(|_| self.text("请检查插件选项。", "Check the plugin options."))?;
        Ok(Some(TaskOptions::Plugin(ndm_core::PluginOptions {
            id: id.clone(),
            values,
        })))
    }

    fn read_plugin_options(
        &self,
        fields: &[Field],
        cx: &App,
    ) -> Result<Map<String, Value>, &'static str> {
        let mut values = Map::new();
        for field in fields {
            let value = if let Some(input) = &field.input {
                let input = input.read(cx).value();
                match &field.definition.control {
                    OptionControl::Integer { .. } if !input.is_empty() => {
                        Some(json!(input.parse::<i64>().map_err(|_| self.text(
                            "请输入有效的插件数值。",
                            "Enter a valid plugin number."
                        ))?))
                    }
                    OptionControl::Text { .. } if !input.is_empty() => Some(json!(input.as_str())),
                    _ => None,
                }
            } else {
                field.value.clone()
            };
            if let Some(value) = value {
                values.insert(field.definition.key.clone(), value);
            }
        }
        Ok(values)
    }

    pub(super) fn remember_engine_choice(&self, source: &reqwest::Url, options: &TaskOptions) {
        if let TaskOptions::Plugin(options) = options
            && self
                .integration
                .draft
                .as_ref()
                .is_some_and(|draft| draft.manual)
        {
            let source = source.clone();
            let registry = self.registry();
            let id = options.id.clone();
            self.runtime.spawn_blocking(move || {
                let _ = registry.remember(&source, &id);
            });
        }
    }

    pub(super) fn engine_draft_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = &self.integration.draft else {
            return div().into_any_element();
        };
        if !draft.ready {
            return div()
                .text_xs()
                .child(self.text("正在匹配下载引擎…", "Looking up download engines…"))
                .into_any_element();
        }
        if draft.candidates.is_empty() {
            return div()
                .when(!draft.native, |row| {
                    row.text_xs().child(self.text(
                        "这个来源需要相应的下载插件。",
                        "Install a plugin for this source from the plugin market.",
                    ))
                })
                .into_any_element();
        }
        let chinese = self.preferences.language == Language::Chinese;
        let tone = crate::motion::ThemeTone::current(cx);
        let selected = draft.selected.clone();
        let current = draft
            .candidates
            .iter()
            .find(|plugin| selected.as_ref() == Some(&plugin.manifest.id));
        let caption = current
            .map(|plugin| plugin.manifest.name.for_language(chinese).to_owned())
            .unwrap_or_else(|| self.text("内置引擎", "Built-in engine").to_owned());
        let mut choices: Vec<_> = draft
            .candidates
            .iter()
            .map(|plugin| {
                (
                    Some(plugin.manifest.id.clone()),
                    plugin.manifest.name.for_language(chinese).to_owned(),
                )
            })
            .collect();
        if draft.native {
            choices.push((None, self.text("内置引擎", "Built-in engine").to_owned()));
        }
        let source = draft.source.clone();
        let view = cx.entity().downgrade();
        let dark = self.is_dark(cx);
        let engine = Button::new("plugin-engine-choice")
            .small()
            .label(caption)
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                choices.iter().fold(menu, |menu, (id, label)| {
                    let view = view.clone();
                    let source = source.clone();
                    let choice = id.clone();
                    menu.item(
                        crate::chrome::choice_item(label.clone(), id == &selected, dark).on_click(
                            move |_, window, cx| {
                                let choice = choice.clone();
                                let source = source.clone();
                                let _ = view.update(cx, |this, cx| {
                                    this.refresh_draft(source, Some(choice), window, cx)
                                });
                            },
                        ),
                    )
                })
            });
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .border_1()
            .border_color(rgb(tone.rgb(0x28344b, 0xe3e8fb)))
            .rounded_lg()
            .p_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(self.text("下载引擎", "Download engine"))
                    .child(engine),
            );
        if draft.panel_error {
            panel = panel.child(div().text_xs().child(self.text(
                "插件面板未能加载。可使用以下默认控件。",
                "Plugin panel could not load. Default controls are available.",
            )));
        }
        panel
            .child(self.plugin_option_controls(&draft.fields, OptionForm::Download, cx))
            .into_any_element()
    }

    fn plugin_option_controls(
        &self,
        fields: &[Field],
        form: OptionForm,
        cx: &mut Context<Self>,
    ) -> Div {
        let chinese = self.preferences.language == Language::Chinese;
        let dark = self.is_dark(cx);
        let disabled = form == OptionForm::Defaults && self.integration.market.busy;
        let segment_group = match form {
            OptionForm::Download => "plugin-download-segments",
            OptionForm::Defaults => "plugin-default-segments",
        };
        let mut panel = div().flex().flex_col().gap_2();
        for (index, field) in fields.iter().enumerate() {
            let label = field.definition.label.for_language(chinese).to_owned();
            let control = match &field.definition.control {
                OptionControl::Boolean => {
                    let enabled = field
                        .value
                        .as_ref()
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    SegmentedControl {
                        id: (segment_group, index).into(),
                        segments: vec![
                            (
                                format!("{}-off", field.definition.key).into(),
                                self.text("关", "Off").into(),
                            ),
                            (
                                format!("{}-on", field.definition.key).into(),
                                self.text("开", "On").into(),
                            ),
                        ],
                        selected: Some(usize::from(enabled)),
                        accessibility_label: Some(label.clone().into()),
                        width: Some(px(104.)),
                        disabled,
                        on_change: std::rc::Rc::new(cx.listener(
                            move |this, selected: &usize, _, cx| {
                                this.set_plugin_value(form, index, json!(*selected == 1), cx);
                            },
                        )),
                    }
                    .into_any_element()
                }
                OptionControl::Choice { choices } => {
                    let value = field.value.as_ref().and_then(Value::as_str);
                    let selected = choices
                        .iter()
                        .position(|choice| Some(choice.value.as_str()) == value);
                    let labels = choices
                        .iter()
                        .map(|choice| choice.label.for_language(chinese));
                    let longest = labels
                        .clone()
                        .map(|label| {
                            label
                                .chars()
                                .map(|ch| if ch.is_ascii() { 1 } else { 2 })
                                .sum::<usize>()
                        })
                        .max()
                        .unwrap_or_default();
                    if (2..=3).contains(&choices.len()) && longest <= 24 {
                        let values: Vec<_> =
                            choices.iter().map(|choice| choice.value.clone()).collect();
                        let button_width = (24. + 7. * longest as f32).max(64.);
                        SegmentedControl {
                            id: (segment_group, index).into(),
                            segments: choices
                                .iter()
                                .zip(labels)
                                .map(|(choice, label)| {
                                    (
                                        format!("{}-{}", field.definition.key, choice.value).into(),
                                        label.to_owned().into(),
                                    )
                                })
                                .collect(),
                            selected,
                            accessibility_label: Some(label.clone().into()),
                            width: Some(px(button_width * choices.len() as f32 + 6.)),
                            disabled,
                            on_change: std::rc::Rc::new(cx.listener(
                                move |this, chosen: &usize, _, cx| {
                                    this.set_plugin_value(form, index, json!(values[*chosen]), cx);
                                },
                            )),
                        }
                        .into_any_element()
                    } else {
                        let menu_items: Vec<_> = choices
                            .iter()
                            .zip(labels)
                            .map(|(choice, label)| (choice.value.clone(), label.to_owned()))
                            .collect();
                        let caption = selected
                            .map(|index| menu_items[index].1.clone())
                            .unwrap_or_else(|| self.text("选择", "Choose").to_owned());
                        let view = cx.entity().downgrade();
                        Button::new(("plugin-choice", index))
                            .small()
                            .disabled(disabled)
                            .label(caption)
                            .dropdown_caret(true)
                            .accessibility_label(label.clone())
                            .dropdown_menu(move |menu, _, _| {
                                menu_items.iter().enumerate().fold(
                                    menu,
                                    |menu, (item, (value, label))| {
                                        let view = view.clone();
                                        let value = value.clone();
                                        menu.item(
                                            crate::chrome::choice_item(
                                                label.clone(),
                                                selected == Some(item),
                                                dark,
                                            )
                                            .on_click(
                                                move |_, _, cx| {
                                                    let _ = view.update(cx, |this, cx| {
                                                        this.set_plugin_value(
                                                            form,
                                                            index,
                                                            json!(value),
                                                            cx,
                                                        );
                                                    });
                                                },
                                            ),
                                        )
                                    },
                                )
                            })
                            .into_any_element()
                    }
                }
                _ => field.input.as_ref().map_or_else(
                    || div().into_any_element(),
                    |input| {
                        Input::new(input)
                            .small()
                            .disabled(disabled)
                            .aria_label(label.clone())
                            .w(px(220.))
                            .into_any_element()
                    },
                ),
            };
            panel = panel.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(div().w(px(156.)).truncate().child(label))
                    .child(control),
            );
        }
        panel
    }

    fn set_plugin_value(
        &mut self,
        form: OptionForm,
        index: usize,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        let fields = match form {
            OptionForm::Download => self
                .integration
                .draft
                .as_mut()
                .map(|draft| &mut draft.fields),
            OptionForm::Defaults => self
                .integration
                .defaults
                .as_mut()
                .map(|editor| &mut editor.fields),
        };
        if let Some(field) = fields.and_then(|fields| fields.get_mut(index)) {
            field.value = Some(value);
            cx.notify();
        }
    }

    pub(super) fn refresh_codex_status(&mut self, cx: &mut Context<Self>) {
        self.codex_action(CodexAction::Inspect, cx);
    }

    fn codex_action(&mut self, action: CodexAction, cx: &mut Context<Self>) {
        if self.integration.codex_busy {
            return;
        }
        self.integration.codex_busy = true;
        let profile = self.store.profile_directory().to_owned();
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let status = runtime.spawn_blocking(move || {
                let target = Target::for_desktop(profile)?;
                match action {
                    CodexAction::Inspect => {}
                    CodexAction::Install => CodexCli.install(&target)?,
                    CodexAction::Remove => CodexCli.remove(&target)?,
                }
                CodexCli.status(&target)
            }).await;
            let _ = view.update(cx, |this, cx| {
                this.integration.codex_busy = false;
                match status {
                    Ok(Ok(status)) => this.integration.codex = Some(status),
                    _ => {
                        this.integration.codex = None;
                        if !matches!(action, CodexAction::Inspect) {
                            this.set_notice(this.text(
                                "无法配置 Codex CLI。请确认已安装 Codex CLI，并检查现有的 ndm-next 配置。",
                                "Could not configure Codex CLI. Check that the CLI is installed and review the existing ndm-next configuration.",
                            ).into(), cx);
                        }
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn ai_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::assets::IconName as AssetIcon;
        let tone = crate::motion::ThemeTone::current(cx);
        let busy = self.integration.codex_busy;
        let status = self.integration.codex;
        let installed = status.is_some_and(|status| status.registered && status.skill_installed);
        let view = cx.entity().downgrade();
        let status_text = match status {
            _ if busy => self.text("处理中…", "Working…"),
            Some(_) if installed => self.text("MCP 与 Skill 已安装", "MCP and Skill installed"),
            Some(_) => self.text("待安装 MCP 与 Skill", "MCP and Skill available to install"),
            None => self.text("安装状态不可用", "Installation status unavailable"),
        };

        crate::view::settings::card_surface(7, tone)
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .border_color(rgb(tone.rgb(0x554681, 0xc9bce9)))
            .bg(linear_gradient(
                112.,
                linear_color_stop(rgb(tone.rgb(0x2b2150, 0xf2eaff)), 0.),
                linear_color_stop(rgb(tone.rgb(0x123542, 0xe7f8fc)), 1.),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(40.))
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgba(tone.rgba(0xb59aff70, 0x885dff48)))
                            .bg(rgba(tone.rgba(0x885dff55, 0x885dff20)))
                            .text_color(rgb(tone.rgb(0xebddff, 0x5936a5)))
                            .child(Icon::new(AssetIcon::Sparkles).with_size(px(20.))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div().text_base().font_weight(FontWeight::SEMIBOLD).child(
                                    self.display_text(self.text("AI 集成", "AI integration")),
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(tone.rgb(0xc2c8e3, 0x455272)))
                                    .child(self.text(
                                        "让 Codex CLI 发起和管理下载",
                                        "Start and manage downloads from Codex CLI",
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgba(tone.rgba(0x0c142b66, 0xffffffb0)))
                            .child(
                                crate::motion::Switch::new("enable-ai-integration")
                                    .checked(self.integration.ai_enabled)
                                    .label(self.text("AI 接入", "AI access"))
                                    .on_click(move |enabled, _, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            if this.store.set_ai_enabled(*enabled).is_ok() {
                                                this.integration.ai_enabled = *enabled;
                                            } else {
                                                this.set_notice(
                                                    this.text(
                                                        "无法保存 AI 设置。",
                                                        "Could not save AI settings.",
                                                    )
                                                    .into(),
                                                    cx,
                                                );
                                            }
                                            cx.notify();
                                        });
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgba(tone.rgba(0xb5acdf40, 0x76689932)))
                    .pt_3()
                    .child(
                        Icon::new(AssetIcon::Terminal)
                            .with_size(px(16.))
                            .text_color(rgb(tone.rgb(0x96e4f1, 0x26687b))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Codex CLI"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if installed && !busy {
                                        tone.rgb(0x8be0c6, 0x226958)
                                    } else {
                                        tone.rgb(0xc2c8e3, 0x455272)
                                    }))
                                    .child(status_text),
                            ),
                    )
                    .child(
                        Button::new("install-codex-integration")
                            .small()
                            .primary()
                            .flex_shrink_0()
                            .disabled(busy)
                            .loading(busy)
                            .label(if installed {
                                self.text("卸载", "Uninstall")
                            } else {
                                self.text("安装", "Install")
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.codex_action(
                                    if installed {
                                        CodexAction::Remove
                                    } else {
                                        CodexAction::Install
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }
}

fn option_fields(
    definitions: Vec<OptionField>,
    defaults: Option<&Map<String, Value>>,
    window: &mut Window,
    cx: &mut Context<Desktop>,
) -> Vec<Field> {
    definitions
        .into_iter()
        .map(|definition| {
            let value = defaults
                .and_then(|defaults| defaults.get(&definition.key).cloned())
                .or_else(|| definition.default.clone());
            let input = matches!(
                definition.control,
                OptionControl::Integer { .. } | OptionControl::Text { .. }
            )
            .then(|| {
                let initial = value.as_ref().map_or_else(String::new, |value| {
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                });
                cx.new(|cx| InputState::new(window, cx).default_value(initial))
            });
            Field {
                definition,
                value,
                input,
            }
        })
        .collect()
}

fn panel_fields(
    plugin: &InstalledPlugin,
    scheme: &str,
    chinese: bool,
) -> std::io::Result<Vec<OptionField>> {
    let Some(script) = plugin.read_panel()? else {
        return Ok(plugin.manifest.options.clone());
    };
    let model = run_panel(
        &script,
        json!({"schema":1,"scheme":scheme,"locale":if chinese { "zh-CN" } else { "en" }}),
    )?;
    plugin.manifest.panel_fields(model)
}

fn run_panel(script: &str, input: Value) -> std::io::Result<Panel> {
    fn script_error(_: rquickjs::Error) -> std::io::Error {
        std::io::Error::other("plugin panel script failed")
    }
    let runtime = rquickjs::Runtime::new().map_err(script_error)?;
    runtime.set_memory_limit(4 * 1024 * 1024);
    runtime.set_max_stack_size(128 * 1024);
    let ctx = rquickjs::Context::full(&runtime).map_err(script_error)?;
    let deadline = Instant::now() + Duration::from_millis(30);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let result = ctx
        .with(|ctx| {
            ctx.globals().set("__ndm_context", input.to_string())?;
            ctx.eval::<(), _>(script)?;
            ctx.eval::<String, _>("JSON.stringify(panel(JSON.parse(__ndm_context)))")
        })
        .map_err(script_error)?;
    if result.len() > 64 * 1024 {
        return Err(std::io::Error::other(
            "plugin panel output exceeded its limit",
        ));
    }
    serde_json::from_str(&result).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::run_panel;
    use serde_json::json;
    #[test]
    fn scripts_can_select_controls_without_running_forever() {
        let model = run_panel(
            "function panel(context) { return {fields: [{key:context.scheme}]}; }",
            json!({"scheme":"quality"}),
        )
        .unwrap();
        assert_eq!(model.fields[0].key, "quality");
        assert!(run_panel("while (true) {}", json!({})).is_err());
    }
}
