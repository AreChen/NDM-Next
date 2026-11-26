use super::*;
use gpui_kit::component::{checkbox::Checkbox, popover::Popover};
use ndm_core::plugins::{SourceFiles, SourceSelection};
use std::collections::BTreeSet;
use tokio_util::sync::{CancellationToken, DropGuard};

#[derive(Default)]
pub(super) enum Preview {
    #[default]
    Idle,
    Loading {
        id: TaskId,
        _cancel_on_close: DropGuard,
    },
    Ready(Selection),
    Failed,
}

pub(super) struct Selection {
    source: Arc<SourceFiles>,
    all: bool,
    exceptions: Arc<BTreeSet<u32>>,
}

impl Selection {
    fn count(&self) -> usize {
        if self.all {
            self.source.files.len() - self.exceptions.len()
        } else {
            self.exceptions.len()
        }
    }
}

impl Desktop {
    pub(in crate::view) fn cancel_file_inspection(&mut self) {
        if let Some(draft) = &mut self.integration.draft
            && matches!(draft.files, Preview::Loading { .. })
        {
            draft.files = Preview::Idle;
        }
    }

    fn inspect_files(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.integration.draft else {
            return;
        };
        let Some(plugin) = draft
            .candidates
            .iter()
            .find(|plugin| {
                Some(&plugin.manifest.id) == draft.selected.as_ref()
                    && plugin.manifest.capabilities.inspect_files
            })
            .cloned()
        else {
            return;
        };
        let Ok(source) = parse_source(&draft.source) else {
            return;
        };
        let values = match self.read_plugin_options(&draft.fields, cx) {
            Ok(values) => values,
            Err(message) => {
                self.set_notice(message.into(), cx);
                return;
            }
        };
        let cancel = CancellationToken::new();
        let id = TaskId::new_v4();
        self.integration.draft.as_mut().unwrap().files = Preview::Loading {
            id,
            _cancel_on_close: cancel.clone().drop_guard(),
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let result = runtime
                .spawn(async move {
                    let mut probe = Box::pin(plugin.inspect_source(source, values, &cancel));
                    tokio::select! {
                        result = &mut probe => result.ok().flatten(),
                        _ = tokio::time::sleep(Duration::from_secs(35)) => {
                            cancel.cancel();
                            let _ = probe.await;
                            None
                        }
                    }
                })
                .await
                .ok()
                .flatten();
            let _ = view.update(cx, |this, cx| {
                if let Some(draft) = this.integration.draft.as_mut()
                    && matches!(draft.files, Preview::Loading { id: running, .. } if running == id)
                {
                    draft.files = result.map_or(Preview::Failed, |source| {
                        Preview::Ready(Selection {
                            source: Arc::new(source),
                            all: true,
                            exceptions: Arc::default(),
                        })
                    });
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn file_selection(
        &self,
        draft: &Draft,
    ) -> Result<Option<SourceSelection>, &'static str> {
        let choice = match &draft.files {
            Preview::Ready(choice) if choice.count() == 0 => {
                return Err(self.text("请至少选择一个文件。", "Select at least one file."));
            }
            Preview::Ready(choice) => choice,
            Preview::Loading { .. } => {
                return Err(self.text("正在读取文件列表。", "Reading the file list."));
            }
            _ => return Ok(None),
        };
        let mut file_ids: Vec<u32> = choice
            .source
            .files
            .iter()
            .filter_map(|file| {
                (choice.all != choice.exceptions.contains(&file.id)).then_some(file.id)
            })
            .collect();
        file_ids.sort_unstable();
        Ok(Some(SourceSelection {
            revision: choice.source.revision.clone(),
            file_ids,
        }))
    }

    pub(super) fn source_picker(&self, draft: &Draft, cx: &mut Context<Self>) -> AnyElement {
        let button = Button::new("pick-source-files")
            .small()
            .icon(IconName::File);
        let choice = match &draft.files {
            Preview::Idle => {
                return button
                    .label(self.text("选择下载文件", "Choose files"))
                    .on_click(cx.listener(|this, _, _, cx| this.inspect_files(cx)))
                    .into_any_element();
            }
            Preview::Loading { .. } => {
                return button
                    .label(self.text("读取文件列表", "Reading files"))
                    .loading(true)
                    .into_any_element();
            }
            Preview::Failed => {
                return button
                    .label(self.text("文件列表读取失败，重试", "File list unavailable. Retry"))
                    .on_click(cx.listener(|this, _, _, cx| this.inspect_files(cx)))
                    .into_any_element();
            }
            Preview::Ready(choice) => choice,
        };
        let caption = self.preferences.language.format(
            "下载文件 {selected} / {total}",
            "Files {selected} / {total}",
            &[
                ("selected", &choice.count().to_string()),
                ("total", &choice.source.files.len().to_string()),
            ],
        );
        let all_label = self.text("全选", "Select all");
        let clear_label = self.text("清空选择", "Clear selection");
        let view = cx.entity().downgrade();
        Popover::new("choose-source-files")
            .trigger(button.label(caption).dropdown_caret(true))
            .default_open(true)
            .content(move |_, _, cx| {
                let Some(desktop) = view.upgrade() else {
                    return div();
                };
                let Some(Preview::Ready(choice)) = desktop
                    .read(cx)
                    .integration
                    .draft
                    .as_ref()
                    .map(|draft| &draft.files)
                else {
                    return div();
                };
                let source = Arc::clone(&choice.source);
                let exceptions = Arc::clone(&choice.exceptions);
                let all = choice.all;
                let count = source.files.len();
                let popup = cx.entity().downgrade();
                let list_popup = popup.clone();
                let list_view = view.clone();
                div()
                    .w(px(472.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().flex().items_center().gap_2().children(
                        [(true, all_label), (false, clear_label)].into_iter().map(
                            |(select, label)| {
                                let view = view.clone();
                                let popup = popup.clone();
                                Button::new(("select-source-files", usize::from(select)))
                                    .small()
                                    .ghost()
                                    .label(label)
                                    .on_click(move |_, _, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            if let Some(Preview::Ready(choice)) = this
                                                .integration
                                                .draft
                                                .as_mut()
                                                .map(|draft| &mut draft.files)
                                            {
                                                choice.all = select;
                                                choice.exceptions = Arc::default();
                                                cx.notify();
                                            }
                                        });
                                        let _ = popup.update(cx, |_, cx| cx.notify());
                                    })
                            },
                        ),
                    ))
                    .child(
                        uniform_list("source-file-list", count, move |range, _, _| {
                            range
                                .map(|index| {
                                    let file = &source.files[index];
                                    let id = file.id;
                                    let view = list_view.clone();
                                    let popup = list_popup.clone();
                                    div()
                                        .h(px(28.))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            Checkbox::new(("source-file", index))
                                                .small()
                                                .checked(all != exceptions.contains(&id))
                                                .accessibility_label(file.path.clone())
                                                .child(
                                                    div()
                                                        .max_w(px(352.))
                                                        .truncate()
                                                        .child(file.path.clone()),
                                                )
                                                .tooltip(file.path.clone())
                                                .on_change(move |checked, _, cx| {
                                                    let _ = view.update(cx, |this, cx| {
                                                        if let Some(Preview::Ready(choice)) = this
                                                            .integration
                                                            .draft
                                                            .as_mut()
                                                            .map(|draft| &mut draft.files)
                                                        {
                                                            let exceptions = Arc::make_mut(
                                                                &mut choice.exceptions,
                                                            );
                                                            if *checked == choice.all {
                                                                exceptions.remove(&id);
                                                            } else {
                                                                exceptions.insert(id);
                                                            }
                                                            cx.notify();
                                                        }
                                                    });
                                                    let _ = popup.update(cx, |_, cx| cx.notify());
                                                }),
                                        )
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .text_xs()
                                                .opacity(0.75)
                                                .child(format_bytes(file.size)),
                                        )
                                })
                                .collect()
                        })
                        .h(px((count.min(7) * 28) as f32))
                        .w_full(),
                    )
            })
            .into_any_element()
    }
}
