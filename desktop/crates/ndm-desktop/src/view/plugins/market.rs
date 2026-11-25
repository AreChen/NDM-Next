use super::*;
use crate::{library::normalize, motion::ThemeTone};
use gpui_kit::assets::IconName;
use ndm_core::plugins::{Localized, PluginManifest};

#[derive(Default)]
pub(super) struct Market {
    pub(super) installed: Vec<InstalledPlugin>,
    pub(super) catalog: Option<Catalog>,
    pub(super) catalog_error: bool,
    pub(super) busy: bool,
    pub(super) package: Option<PackageOperation>,
    entries: Vec<Entry>,
    visible: Arc<[usize]>,
    query: String,
    category: Option<String>,
    installed_only: bool,
    scroll: UniformListScrollHandle,
}

pub(super) struct PackageOperation {
    pub(super) name: Option<Localized>,
    pub(super) updating: bool,
    pub(super) progress: tokio::sync::watch::Receiver<PackageProgress>,
}

struct Entry {
    installed: Option<usize>,
    listing: Option<usize>,
    searchable: String,
}

impl Market {
    pub(super) fn rebuild(&mut self) {
        let catalog = self.catalog.as_ref();
        let listings = catalog.map_or(&[][..], |catalog| catalog.plugins.as_slice());
        let listings_by_id: HashMap<_, _> = listings
            .iter()
            .enumerate()
            .map(|(index, plugin)| (plugin.id.as_str(), index))
            .collect();
        let installed_ids: HashSet<_> = self
            .installed
            .iter()
            .map(|plugin| plugin.manifest.id.as_str())
            .collect();
        let installed = self.installed.iter().enumerate().map(|(index, plugin)| {
            (
                Some(index),
                listings_by_id.get(plugin.manifest.id.as_str()).copied(),
            )
        });
        let uninstalled = listings
            .iter()
            .enumerate()
            .filter(|(_, plugin)| !installed_ids.contains(plugin.id.as_str()))
            .map(|(index, _)| (None, Some(index)));

        self.entries = installed
            .chain(uninstalled)
            .map(|(installed, listing)| {
                let mut fields = Vec::new();
                if let Some(index) = installed {
                    let manifest = &self.installed[index].manifest;
                    fields.extend([
                        manifest.id.as_str(),
                        &manifest.name.en,
                        &manifest.name.zh,
                        &manifest.description.en,
                        &manifest.description.zh,
                    ]);
                }
                if let Some(index) = listing {
                    let plugin = &listings[index];
                    fields.extend([
                        plugin.id.as_str(),
                        &plugin.name.en,
                        &plugin.name.zh,
                        &plugin.description.en,
                        &plugin.description.zh,
                    ]);
                    fields.extend(plugin.keywords.iter().map(String::as_str));
                    if let Some((_, label)) = catalog.and_then(|c| plugin_category(c, plugin)) {
                        fields.extend([label.en.as_str(), label.zh.as_str()]);
                    }
                }
                Entry {
                    installed,
                    listing,
                    searchable: normalize(&fields.join("\n")),
                }
            })
            .collect();
        if self
            .category
            .as_ref()
            .is_some_and(|id| !catalog.is_some_and(|catalog| catalog.categories.contains_key(id)))
        {
            self.category = None;
        }
        self.refilter();
    }

    fn listing(&self, entry: &Entry) -> Option<&CatalogPlugin> {
        self.catalog
            .as_ref()
            .zip(entry.listing)
            .map(|(catalog, index)| &catalog.plugins[index])
    }

    fn refilter(&mut self) {
        let query = normalize(&self.query);
        let terms: Vec<_> = query.split_whitespace().collect();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                (!self.installed_only || entry.installed.is_some())
                    && self.category.as_deref().is_none_or(|id| {
                        self.listing(entry)
                            .and_then(|item| item.category.as_deref())
                            == Some(id)
                    })
                    && terms.iter().all(|term| entry.searchable.contains(term))
            })
            .map(|(index, _)| index)
            .collect();
        if !self.visible.is_empty() {
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
    }

    fn filtered(&self) -> bool {
        !self.query.trim().is_empty() || self.category.is_some() || self.installed_only
    }
}

fn plugin_category<'a>(
    catalog: &'a Catalog,
    listing: &CatalogPlugin,
) -> Option<(&'a str, &'a Localized)> {
    catalog
        .categories
        .get_key_value(listing.category.as_deref()?)
        .map(|(key, label)| (key.as_str(), label))
}

pub(in crate::view) fn market_search_placeholder(language: Language) -> &'static str {
    match language {
        Language::Chinese => "搜索插件名称或关键词",
        Language::English => "Search plugins by name or keyword",
    }
}

impl Desktop {
    pub(in crate::view) fn search_market(&mut self, query: &str, cx: &mut Context<Self>) {
        let market = &mut self.integration.market;
        if market.query != query {
            market.query = query.to_owned();
            market.refilter();
            cx.notify();
        }
    }

    pub(in crate::view) fn market_panel(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        if let Some(editor) = &self.integration.defaults {
            return self.plugin_defaults_panel(editor, cx);
        }
        let tone = ThemeTone::current(cx);
        let market = &self.integration.market;
        let busy = market.busy || matches!(self.update_state, UpdateState::Installing);
        let mut panel = div()
            .id("plugin-market")
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(market_icon(IconName::Store, false, tone))
                    .child(
                        div()
                            .ml_1()
                            .flex_1()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .child(self.display_text(self.text("插件市场", "Plugin market"))),
                    )
                    .child(
                        crate::chrome::icon_button(
                            "reload-market",
                            IconName::RefreshCw,
                            self.text("刷新市场", "Refresh market"),
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_plugin_market(cx))),
                    )
                    .child(
                        Button::new("import-plugin")
                            .small()
                            .icon(IconName::FolderOpen)
                            .label(self.text("导入插件", "Import plugin"))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_plugin_package(window, cx)
                            })),
                    ),
            )
            .child(self.market_filters(tone, cx));

        if let Some(package) = &market.package {
            panel = panel.child(self.package_progress(package, tone));
        } else if market.busy || market.catalog_error {
            let (icon, message) = if market.busy {
                (
                    IconName::RefreshCw,
                    self.text("正在处理插件…", "Working on plugins…"),
                )
            } else {
                (
                    IconName::CircleAlert,
                    self.text(
                        "暂时无法加载市场目录。",
                        "The market catalog is unavailable.",
                    ),
                )
            };
            panel = panel.child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .bg(rgb(tone.rgb(0x1c263b, 0xeaf0fa)))
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(rgb(tone.rgb(0xb4c2dc, 0x4f607b)))
                    .child(Icon::new(icon).with_size(px(14.)))
                    .child(message),
            );
        }

        let visible = Arc::clone(&market.visible);
        if !visible.is_empty() {
            let columns = if width >= 656. { 2 } else { 1 };
            panel = panel.child(
                uniform_list(
                    "plugin-market-cards",
                    visible.len().div_ceil(columns),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|row| {
                                let start = row * columns;
                                let items = &visible[start..(start + columns).min(visible.len())];
                                div()
                                    .w_full()
                                    .h(px(164.))
                                    .pb_3()
                                    .flex()
                                    .gap_3()
                                    .children(
                                        items
                                            .iter()
                                            .map(|&index| this.market_card(index, tone, busy, cx)),
                                    )
                                    .when(items.len() < columns, |row| row.child(div().flex_1()))
                                    .into_any_element()
                            })
                            .collect()
                    }),
                )
                .track_scroll(&market.scroll)
                .flex_1()
                .min_h_0(),
            );
        } else if !market.busy && (market.filtered() || !market.catalog_error) {
            let filtered = market.filtered();
            panel = panel.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_sm()
                    .text_color(rgb(tone.rgb(0xa3b1ca, 0x586986)))
                    .child(market_icon(
                        if filtered {
                            IconName::SearchX
                        } else {
                            IconName::PackageOpen
                        },
                        false,
                        tone,
                    ))
                    .child(if filtered {
                        self.text("没有符合条件的插件", "No matching plugins")
                    } else {
                        self.text("市场暂无插件", "No plugins in the market yet")
                    })
                    .when(filtered, |empty| {
                        empty.child(
                            Button::new("clear-market-filters")
                                .small()
                                .ghost()
                                .label(self.text("清除筛选", "Clear filters"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let market = &mut this.integration.market;
                                    market.query.clear();
                                    market.category = None;
                                    market.installed_only = false;
                                    market.refilter();
                                    this.market_search_input
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    cx.notify();
                                })),
                        )
                    }),
            );
        }
        panel.into_any_element()
    }

    fn package_progress(&self, package: &PackageOperation, tone: ThemeTone) -> Div {
        use gpui_kit::component::spinner::Spinner;

        let (phase, bytes) = match *package.progress.borrow() {
            PackageProgress::Downloading { received, total } => (
                self.text("正在下载", "Downloading"),
                Some((received, total)),
            ),
            PackageProgress::Verifying => (self.text("正在校验安装包", "Verifying package"), None),
            PackageProgress::Installing { extracted, total } => (
                self.text("正在解包", "Extracting"),
                Some((extracted, total)),
            ),
        };
        let ratio = bytes.and_then(|(done, total)| {
            total
                .filter(|total| *total > 0)
                .map(|total| (done as f32 / total as f32).clamp(0., 1.))
        });
        let description = match bytes {
            Some((done, Some(total))) => {
                format!("{phase} · {} / {}", format_bytes(done), format_bytes(total))
            }
            Some((done, None)) => format!("{phase} · {}", format_bytes(done)),
            None => phase.into(),
        };
        let chinese = self.preferences.language == Language::Chinese;
        let action = if package.updating {
            self.text("更新", "Updating")
        } else {
            self.text("安装", "Installing")
        };
        let label = package.name.as_ref().map_or_else(
            || self.text("导入插件", "Importing plugin").into(),
            |name| format!("{action} {}", name.for_language(chinese)),
        );
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_2()
            .rounded_md()
            .bg(rgb(tone.rgb(0x1c263b, 0xeaf0fa)))
            .px_3()
            .py_2()
            .text_xs()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(ratio.is_none(), |row| row.child(Spinner::new().small()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(tone.rgb(0xb4c2dc, 0x4f607b)))
                            .child(description),
                    )
                    .when_some(ratio, |row, ratio| {
                        row.child(format!("{:.0}%", (ratio * 100.).floor()))
                    }),
            )
            .when_some(ratio, |panel, ratio| {
                panel.child(
                    div()
                        .w_full()
                        .h(px(4.))
                        .rounded_full()
                        .bg(rgb(tone.rgb(0x33415d, 0xdce1f0)))
                        .child(
                            div()
                                .h_full()
                                .w(relative(ratio))
                                .rounded_full()
                                .bg(rgb(0x8864ed)),
                        ),
                )
            })
    }

    fn market_filters(&self, tone: ThemeTone, cx: &mut Context<Self>) -> Div {
        let market = &self.integration.market;
        let chinese = self.preferences.language == Language::Chinese;
        let all_categories = self.text("全部分类", "All categories");
        let mut categories = vec![(None, SharedString::from(all_categories))];
        if let Some(catalog) = &market.catalog {
            categories.extend(catalog.categories.iter().map(|(id, label)| {
                (
                    Some(id.clone()),
                    SharedString::from(label.for_language(chinese).to_owned()),
                )
            }));
        }
        let current = market.category.clone();
        let label = categories
            .iter()
            .find(|(id, _)| *id == current)
            .map_or(SharedString::from(all_categories), |(_, label)| {
                label.clone()
            });
        let view = cx.entity().downgrade();
        let dark = self.is_dark(cx);
        let input = self.market_search_input.clone();
        let page_focus = self.page_focus.clone();
        div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap_2()
            .child(
                div()
                    .id("market-search-container")
                    .flex_1()
                    .min_w_0()
                    .on_mouse_down_out(move |_, window, cx| {
                        if input.read(cx).focus_handle(cx).is_focused(window) {
                            page_focus.focus(window, cx);
                        }
                    })
                    .child(
                        Input::new(&self.market_search_input)
                            .id("search-plugins")
                            .small()
                            .w_full()
                            .prefix(Icon::new(IconName::Search).small())
                            .cleanable(true)
                            .aria_label(market_search_placeholder(self.preferences.language)),
                    ),
            )
            .child(
                Button::new("market-category")
                    .small()
                    .flex_shrink_0()
                    .max_w(px(184.))
                    .label(label)
                    .dropdown_caret(true)
                    .accessibility_label(self.text("插件分类", "Plugin category"))
                    .dropdown_menu(move |menu, _, _| {
                        categories.iter().cloned().fold(menu, |menu, (id, label)| {
                            let view = view.clone();
                            menu.item(
                                crate::chrome::choice_item(label, id == current, dark).on_click(
                                    move |_, _, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            this.integration.market.category = id.clone();
                                            this.integration.market.refilter();
                                            cx.notify();
                                        });
                                    },
                                ),
                            )
                        })
                    }),
            )
            .child(
                Button::new("market-installed")
                    .small()
                    .flex_shrink_0()
                    .icon(IconName::CircleCheck)
                    .label(self.text("仅已安装", "Installed only"))
                    .when(market.installed_only, |button| {
                        button
                            .bg(rgb(tone.rgb(0x302b50, 0xede9ff)))
                            .text_color(rgb(tone.rgb(0xd9d1ff, 0x523fa7)))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        let market = &mut this.integration.market;
                        market.installed_only = !market.installed_only;
                        market.refilter();
                        cx.notify();
                    })),
            )
    }

    fn market_card(
        &self,
        index: usize,
        tone: ThemeTone,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let market = &self.integration.market;
        let entry = &market.entries[index];
        let listing = market.listing(entry);
        let installed_plugin = entry.installed.map(|i| &market.installed[i]);
        let installed = installed_plugin.map(|plugin| &plugin.manifest);
        let (name, description) = if let Some(plugin) = listing {
            (&plugin.name, &plugin.description)
        } else {
            let plugin = installed.expect("a local entry is installed");
            (&plugin.name, &plugin.description)
        };
        let version = installed.map_or_else(
            || &listing.expect("an uninstalled entry is listed").version,
            |plugin| &plugin.version,
        );
        let category = market
            .catalog
            .as_ref()
            .zip(listing)
            .and_then(|(catalog, plugin)| plugin_category(catalog, plugin));
        let chinese = self.preferences.language == Language::Chinese;
        let (icon, cyan) = match category.map(|(id, _)| id) {
            Some("video-audio") => (IconName::Film, false),
            Some("torrents") => (IconName::Network, true),
            _ => (IconName::Puzzle, false),
        };
        let category_label = category.map_or_else(
            || {
                if listing.is_some() {
                    self.text("其他", "Other")
                } else {
                    self.text("本地插件", "Local plugin")
                }
            },
            |(_, label)| label.for_language(chinese),
        );
        let muted = rgb(tone.rgb(0xa3b1ca, 0x586986));
        let title = name.for_language(chinese).to_owned();
        let summary = description.for_language(chinese).to_owned();
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(tone.rgb(0x33415d, 0xdce1f0)))
            .bg(linear_gradient(
                115.,
                linear_color_stop(
                    rgb(if cyan {
                        tone.rgb(0x172932, 0xf0fafb)
                    } else {
                        tone.rgb(0x202039, 0xfaf7ff)
                    }),
                    0.,
                ),
                linear_color_stop(rgb(tone.rgb(0x141c2c, 0xf7f9fd)), 1.),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(market_icon(icon, cyan, tone))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(category_label.to_owned()),
                            ),
                    )
                    .when(installed_plugin.is_some(), |header| {
                        header.child(
                            div()
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .gap_1()
                                .py_0p5()
                                .px_2()
                                .rounded_full()
                                .bg(rgb(tone.rgb(0x203b3a, 0xdff4ee)))
                                .text_color(rgb(tone.rgb(0x8be0c6, 0x226958)))
                                .text_xs()
                                .child(Icon::new(IconName::CircleCheck).with_size(px(12.)))
                                .child(self.text("已安装", "Installed")),
                        )
                    }),
            )
            .child(
                div()
                    .h(px(34.))
                    .overflow_hidden()
                    .line_height(px(17.))
                    .line_clamp(2)
                    .text_sm()
                    .text_color(muted)
                    .child(summary),
            )
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("v{version}")),
                    )
                    .child(self.market_actions(index, installed, listing, busy, cx)),
            )
    }

    fn market_actions(
        &self,
        index: usize,
        installed: Option<&PluginManifest>,
        listing: Option<&CatalogPlugin>,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let actions = div().flex().items_center().gap_2();
        let candidate = listing.filter(|plugin| plugin.artifact().is_some());
        if let Some(installed) = installed {
            let settings_id = installed.id.clone();
            let id = installed.id.clone();
            return actions
                .child(
                    Button::new(("plugin-settings", index))
                        .small()
                        .ghost()
                        .disabled(busy)
                        .icon(IconName::Settings2)
                        .label(self.text("设置", "Settings"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_plugin_defaults(&settings_id, window, cx);
                        })),
                )
                .when_some(
                    candidate
                        .filter(|plugin| plugin.version > installed.version)
                        .cloned(),
                    |actions, update| {
                        actions.child(
                            Button::new(("update-plugin", index))
                                .small()
                                .primary()
                                .disabled(busy)
                                .label(self.text("更新", "Update"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.package_action(
                                        PackageAction::Install(Box::new(update.clone())),
                                        cx,
                                    )
                                })),
                        )
                    },
                )
                .child(
                    Button::new(("remove-plugin", index))
                        .small()
                        .ghost()
                        .disabled(busy)
                        .label(self.text("卸载", "Uninstall"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.package_action(PackageAction::Remove(id.clone()), cx)
                        })),
                );
        }
        if let Some(candidate) = candidate.cloned() {
            actions.child(
                Button::new(("install-market-plugin", index))
                    .small()
                    .primary()
                    .disabled(busy)
                    .icon(IconName::ArrowDownToLine)
                    .label(self.text("安装", "Install"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.package_action(PackageAction::Install(Box::new(candidate.clone())), cx)
                    })),
            )
        } else {
            actions.child(
                div()
                    .text_xs()
                    .child(self.text("此平台暂不可用", "Unavailable on this platform")),
            )
        }
    }
}

fn market_icon(icon: IconName, cyan: bool, tone: ThemeTone) -> Div {
    let (bg, fg) = if cyan {
        (tone.rgb(0x20414d, 0xd9f2f5), tone.rgb(0x7bddf2, 0x135d70))
    } else {
        (tone.rgb(0x353059, 0xede6ff), tone.rgb(0xd4c7ff, 0x5942ae))
    };
    div()
        .size(px(38.))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_lg()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .child(Icon::new(icon).with_size(px(19.)))
}

#[cfg(test)]
mod tests {
    use super::Market;
    use ndm_core::plugins::{InstalledPlugin, PluginManifest};
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn search_combines_both_languages_category_and_installation_filters() {
        let catalog = serde_json::from_value(json!({
            "schema": 1,
            "categories": {
                "torrents": {"en": "Torrents", "zh": "种子与磁力"},
                "video-audio": {"en": "Video & audio", "zh": "视频与音频"}
            },
            "plugins": [
                {
                    "id": "bittorrent",
                    "version": "1.0.0",
                    "name": {"en": "BitTorrent"},
                    "description": {"en": "Magnet link downloads", "zh": "下载磁链"},
                    "category": "torrents",
                    "keywords": ["BT"],
                    "artifacts": {}
                },
                {
                    "id": "yt-dlp",
                    "version": "1.0.0",
                    "name": {"en": "YouTube"},
                    "description": {"en": "Download a video"},
                    "category": "video-audio",
                    "artifacts": {}
                }
            ]
        }))
        .unwrap();
        let mut manifest: PluginManifest = serde_json::from_str(include_str!(
            "../../../../ndm-plugin-sdk/examples/file-copy.json"
        ))
        .unwrap();
        manifest.id = "bittorrent".into();
        let mut market = Market {
            catalog: Some(catalog),
            installed: vec![InstalledPlugin {
                manifest,
                directory: PathBuf::new(),
                option_defaults: Default::default(),
                ai_enabled: true,
            }],
            ..Market::default()
        };
        market.rebuild();
        assert_eq!(market.visible.len(), 2);
        market.query = "ＢＴ 磁力".into();
        market.category = Some("torrents".into());
        market.installed_only = true;
        market.refilter();
        let result = &market.entries[market.visible[0]];
        assert_eq!(market.visible.len(), 1);
        assert_eq!(market.listing(result).unwrap().id, "bittorrent");

        market.category = Some("video-audio".into());
        market.refilter();
        assert!(market.visible.is_empty());
    }
}
