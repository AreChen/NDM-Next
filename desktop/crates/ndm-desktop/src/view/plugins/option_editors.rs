use super::*;
use gpui_kit::component::{checkbox::Checkbox, popover::Popover};
use ndm_core::plugins::selected_text_values;

impl Desktop {
    pub(super) fn plugin_option_label(&self, index: usize, field: &Field, label: String) -> Div {
        let help = self.option_help(field);
        div()
            .w(px(156.))
            .flex()
            .items_center()
            .gap_1()
            .child(div().min_w_0().truncate().child(label))
            .when_some(help, |row, help| {
                row.child(
                    Button::new(("plugin-option-help", index))
                        .ghost()
                        .size(px(16.))
                        .flex_shrink_0()
                        .icon(Icon::new(IconName::Info).with_size(px(13.)))
                        .tooltip(help)
                        .accessibility_label(self.text("填写说明", "Option help")),
                )
            })
    }

    fn option_help(&self, field: &Field) -> Option<SharedString> {
        field.definition.help.as_ref().map(|help| {
            self.preferences
                .language
                .plugin_text(help)
                .to_owned()
                .into()
        })
    }

    pub(super) fn proxy_address_editor(
        &self,
        form: OptionForm,
        index: usize,
        field: &Field,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(TextEditor::ProxyAddress { schemes, .. }), Some(input)) =
            (&field.definition.editor, &field.input)
        else {
            return div().into_any_element();
        };
        let selected = field.proxy_scheme.as_ref().unwrap_or(&schemes[0]).clone();
        let view = cx.entity().downgrade();
        let choices = schemes.clone();
        let dark = self.is_dark(cx);
        let protocol = Button::new(("plugin-proxy-protocol", index))
            .small()
            .disabled(disabled)
            .label(selected.to_ascii_uppercase())
            .dropdown_caret(true)
            .accessibility_label(self.text("代理协议", "Proxy protocol"))
            .when_some(self.option_help(field), Button::tooltip)
            .dropdown_menu(move |menu, _, _| {
                choices.iter().fold(menu, |menu, scheme| {
                    let view = view.clone();
                    let choice = scheme.clone();
                    menu.item(
                        crate::chrome::choice_item(
                            scheme.to_ascii_uppercase(),
                            *scheme == selected,
                            dark,
                        )
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                if let Some(field) = this.plugin_field_mut(form, index) {
                                    field.proxy_scheme = Some(choice.clone());
                                    cx.notify();
                                }
                            });
                        }),
                    )
                })
            });
        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(protocol)
            .child(
                Input::new(input)
                    .small()
                    .disabled(disabled)
                    .w(px(248.))
                    .aria_label(self.text("代理地址", "Proxy address")),
            )
            .into_any_element()
    }

    pub(super) fn multiselect_editor(
        &self,
        form: OptionForm,
        index: usize,
        field: &Field,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(TextEditor::MultiSelect {
            choices,
            max_selected,
        }) = &field.definition.editor
        else {
            return div().into_any_element();
        };
        let selected = field_selection(field);
        let language = self.preferences.language;
        let caption = if selected.is_empty() {
            self.text("选择", "Select").to_owned()
        } else {
            language.format(
                "已选 {count} 项",
                "{count} selected",
                &[("count", &selected.len().to_string())],
            )
        };
        let trigger = Button::new(("plugin-multiselect", index))
            .small()
            .disabled(disabled)
            .label(caption)
            .dropdown_caret(true)
            .accessibility_label(language.plugin_text(&field.definition.label).to_owned());
        if disabled {
            return trigger.into_any_element();
        }
        let view = cx.entity().downgrade();
        let limit = *max_selected;
        let choices: Vec<_> = choices
            .iter()
            .map(|choice| {
                (
                    choice.value.clone(),
                    language.plugin_text(&choice.label).to_owned(),
                )
            })
            .collect();
        let prefix = if form == OptionForm::Defaults {
            "plugin-default-checklist"
        } else {
            "plugin-download-checklist"
        };
        Popover::new((prefix, index))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let selected: Vec<String> = view
                    .upgrade()
                    .and_then(|view| {
                        let editor = &view.read(cx).integration;
                        let fields = match form {
                            OptionForm::Download => {
                                editor.draft.as_ref().map(|draft| &draft.fields)
                            }
                            OptionForm::Defaults => {
                                editor.defaults.as_ref().map(|editor| &editor.fields)
                            }
                        };
                        fields.and_then(|fields| fields.get(index)).map(|field| {
                            field_selection(field)
                                .into_iter()
                                .map(str::to_owned)
                                .collect()
                        })
                    })
                    .unwrap_or_default();
                let mut items = choices.clone();
                for value in &selected {
                    if !choices.iter().any(|(choice, _)| choice == value) {
                        items.push((value.clone(), value.clone()));
                    }
                }
                let height = (items.len().min(7) * 28) as f32;
                let view = view.clone();
                let popup = cx.entity().downgrade();
                uniform_list("plugin-checklist-items", items.len(), move |range, _, _| {
                    range
                        .map(|row| {
                            let (value, label) = &items[row];
                            let checked = selected.contains(value);
                            let value = value.clone();
                            let view = view.clone();
                            let popup = popup.clone();
                            div().h(px(28.)).flex().items_center().child(
                                Checkbox::new(("plugin-checklist-choice", row))
                                    .small()
                                    .label(label.clone())
                                    .checked(checked)
                                    .disabled(!checked && selected.len() >= limit)
                                    .on_change(move |checked, _, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            if let Some(field) = this.plugin_field_mut(form, index)
                                            {
                                                let mut selected = field_selection(field);
                                                selected.retain(|item| *item != value);
                                                if *checked && selected.len() < limit {
                                                    selected.push(&value);
                                                }
                                                field.value = Some(json!(selected.join(",")));
                                                cx.notify();
                                            }
                                        });
                                        let _ = popup.update(cx, |_, cx| cx.notify());
                                    }),
                            )
                        })
                        .collect()
                })
                .h(px(height))
                .w(px(244.))
            })
            .into_any_element()
    }
}

fn field_selection(field: &Field) -> Vec<&str> {
    field
        .value
        .as_ref()
        .and_then(Value::as_str)
        .map(selected_text_values)
        .unwrap_or_default()
}
