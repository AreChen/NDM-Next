use crate::{chrome, preferences::Language, view::Desktop};
use gpui_kit::{
    component::{
        Sizable, Theme as ComponentTheme, ThemeMode,
        button::{Button, ButtonVariants},
    },
    prelude::*,
    *,
};
use ndm_core::{Task, TaskId};
use std::path::PathBuf;

pub(crate) struct FilePreview {
    name: String,
    directory: PathBuf,
}

impl FilePreview {
    pub(crate) fn from_task(task: &Task) -> Self {
        Self {
            name: crate::filenames::display_name(task).unwrap_or_else(|| "—".into()),
            directory: task.request.destination.clone(),
        }
    }
}

pub(crate) fn open(
    ids: Vec<TaskId>,
    preview: Option<FilePreview>,
    language: Language,
    parent: WeakEntity<Desktop>,
    cx: &mut App,
) -> Result<AnyWindowHandle, ()> {
    let height = dialog_height(preview.is_some());
    let bounds = Bounds::centered(None, size(px(480.), px(height)), cx);
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            kind: WindowKind::Dialog,
            is_resizable: false,
            is_minimizable: false,
            ..chrome::window_options()
        },
        cx,
        move |window, cx| {
            chrome::configure_window_chrome(window);
            chrome::fit_content(window, size(px(480.), px(height)), cx);
            window.set_window_title("NDM Next · Delete");
            let handle = window.window_handle();
            cx.new(|cx| {
                cx.on_release(move |this: &mut ConfirmDelete, cx| {
                    let _ = this.parent.update(cx, |parent, cx| {
                        parent.delete_confirmation_closed(handle, cx);
                    });
                })
                .detach();
                ConfirmDelete {
                    ids,
                    preview,
                    folder_error: false,
                    language,
                    parent,
                    focus: chrome::page_focus(window, cx),
                }
            })
        },
    )
    .map_err(|_| ())?;
    Ok(handle)
}

struct ConfirmDelete {
    ids: Vec<TaskId>,
    preview: Option<FilePreview>,
    folder_error: bool,
    language: Language,
    parent: WeakEntity<Desktop>,
    focus: FocusHandle,
}

impl ConfirmDelete {
    fn text(&self, chinese: &'static str, english: &'static str) -> &'static str {
        if self.language == Language::Chinese {
            chinese
        } else {
            english
        }
    }

    fn confirm(&mut self, delete_files: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.ids.clone();
        let _ = self
            .parent
            .update(cx, |parent, cx| parent.delete_tasks(ids, delete_files, cx));
        window.remove_window();
    }
}

impl Render for ConfirmDelete {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = matches!(ComponentTheme::global(cx).mode, ThemeMode::Dark);
        let tone = crate::motion::ThemeTone::current(cx);
        let title = if self.ids.len() == 1 {
            self.text("删除下载任务？", "Delete this download?")
                .to_owned()
        } else {
            format!(
                "{} {} {}",
                self.text("删除", "Delete"),
                self.ids.len(),
                self.text("个下载任务？", "downloads?")
            )
        };
        chrome::blank_drag_region("delete-background", window, cx)
            .size_full()
            .child(delete_dialog(
                title,
                self.preview.as_ref().map(|preview| {
                    file_preview(
                        preview.name.clone(),
                        preview.directory.to_string_lossy().into_owned(),
                        self.language == Language::Chinese,
                        dark,
                        self.folder_error,
                        cx.listener(|this, _, _, cx| {
                            if let Some(preview) = &this.preview {
                                let directory = preview.directory.clone();
                                crate::files::open_after_click(
                                    cx,
                                    move || crate::files::open_directory(&directory),
                                    |this, result, cx| {
                                        this.folder_error = result.is_err();
                                        cx.notify();
                                    },
                                );
                            }
                        }),
                    )
                    .into_any_element()
                }),
                self.language == Language::Chinese,
                tone,
                cx.listener(|_, _, window, _| window.remove_window()),
                cx.listener(|this, _, window, cx| this.confirm(false, window, cx)),
                cx.listener(|this, _, window, cx| this.confirm(true, window, cx)),
            ))
            .track_focus(&self.focus)
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    window.remove_window();
                }
            })
    }
}

fn dialog_height(has_name: bool) -> f32 {
    if has_name { 144. } else { 112. }
}

fn delete_dialog(
    title: String,
    preview: Option<AnyElement>,
    zh: bool,
    tone: impl Into<crate::motion::ThemeTone>,
    cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    keep: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    delete: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let tone = tone.into();
    div()
        .id("delete-confirmation")
        .size_full()
        .flex()
        .flex_col()
        .bg(chrome::floating_surface(tone))
        .text_color(rgb(tone.rgb(0xf0f3ff, 0x19253b)))
        .child(chrome::floating_header(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(tone.rgb(0xf0f3ff, 0x19253b)))
                .child(title),
            tone,
            zh,
            false,
        ))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .px_4()
                .pt_1()
                .pb_4()
                .flex()
                .flex_col()
                .justify_end()
                .gap_2()
                .when_some(preview, |body, preview| body.child(preview))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(32.))
                        .flex_shrink_0()
                        .gap_2()
                        .child(
                            Button::new("delete-cancel")
                                .small()
                                .ghost()
                                .h(px(32.))
                                .label(if zh { "取消" } else { "Cancel" })
                                .debug_selector(|| "delete-cancel".into())
                                .on_click(cancel),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("delete-keep")
                                .small()
                                .h(px(32.))
                                .label(if zh { "仅删除记录" } else { "Keep files" })
                                .debug_selector(|| "delete-keep".into())
                                .on_click(keep),
                        )
                        .child(
                            Button::new("delete-files")
                                .small()
                                .danger()
                                .h(px(32.))
                                .icon(gpui_kit::component::IconName::Delete)
                                .label(if zh {
                                    "删除文件和记录"
                                } else {
                                    "Delete files"
                                })
                                .debug_selector(|| "delete-files".into())
                                .on_click(delete),
                        ),
                ),
        )
}

fn file_preview(
    name: String,
    directory: String,
    zh: bool,
    dark: bool,
    failed: bool,
    open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let path_label = if failed {
        format!(
            "{} · {directory}",
            if zh {
                "无法打开文件夹"
            } else {
                "Could not open folder"
            }
        )
    } else {
        directory.clone()
    };
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .min_w_0()
        .gap(px(2.))
        .child(
            div()
                .id("delete-filename")
                .debug_selector(|| "delete-filename".into())
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .truncate()
                .child(name),
        )
        .child(
            div()
                .flex()
                .items_center()
                .h(px(28.))
                .gap_2()
                .child(
                    div()
                        .id("delete-location")
                        .debug_selector(|| "delete-location".into())
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .truncate()
                        .tooltip(move |_, cx| {
                            cx.new(|_| {
                                gpui_kit::component::tooltip::Tooltip::new(directory.clone())
                            })
                            .into()
                        })
                        .text_color(rgb(if failed {
                            0xd96779
                        } else if dark {
                            0x8999b4
                        } else {
                            0x6e7d98
                        }))
                        .child(path_label),
                )
                .child(
                    chrome::icon_button(
                        "delete-open-folder",
                        gpui_kit::component::IconName::FolderOpen,
                        if zh { "打开文件夹" } else { "Open folder" },
                    )
                    .debug_selector(|| "delete-open-folder".into())
                    .on_click(open),
                ),
        )
}

#[cfg(test)]
mod tests {
    use gpui::{Context, IntoElement, Modifiers, Render, Window, px, size};
    use gpui_kit as gpui;
    use std::{cell::Cell, rc::Rc};

    struct Harness {
        zh: bool,
        has_name: bool,
        clicked: Rc<Cell<u8>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let cancel = self.clicked.clone();
            let keep = self.clicked.clone();
            let delete = self.clicked.clone();
            let folder = self.clicked.clone();
            super::delete_dialog(
                if self.zh {
                    "删除下载任务？"
                } else {
                    "Delete downloads?"
                }
                .into(),
                self.has_name.then(|| {
                    super::file_preview(
                        "一个非常长的下载文件名-a-long-download-name.zip".into(),
                        "C:/Downloads/下载文件/a-very-long-directory-name/another-folder".into(),
                        self.zh,
                        true,
                        false,
                        move |_, _, _| folder.set(4),
                    )
                    .into_any_element()
                }),
                self.zh,
                true,
                move |_, _, _| cancel.set(1),
                move |_, _, _| keep.set(2),
                move |_, _, _| delete.set(3),
            )
        }
    }

    #[gpui::test]
    fn confirmation_actions_fit_and_receive_clicks_in_both_languages(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui::init);
        let clicked = Rc::new(Cell::new(0));
        let (view, cx) = cx.add_window_view({
            let clicked = clicked.clone();
            move |_, _| Harness {
                zh: true,
                has_name: true,
                clicked,
            }
        });
        for scale in [1., 1.25, 1.5, 2.] {
            cx.simulate_scale_factor_change(scale);
            for (zh, has_name) in [(true, true), (false, true), (true, false), (false, false)] {
                let height = super::dialog_height(has_name);
                cx.simulate_resize(size(px(480.), px(height)));
                cx.update(|window, cx| {
                    view.update(cx, |this, cx| {
                        this.zh = zh;
                        this.has_name = has_name;
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                #[cfg(target_os = "macos")]
                {
                    assert!(cx.debug_bounds("minimize").is_none());
                    assert!(cx.debug_bounds("zoom").is_none());
                    let close = cx.debug_bounds("close").unwrap();
                    assert_eq!(close.right(), px(480.));
                    assert!(close.bottom() <= px(34.));
                }
                if has_name {
                    let name = cx.debug_bounds("delete-filename").unwrap();
                    let location = cx.debug_bounds("delete-location").unwrap();
                    let folder = cx.debug_bounds("delete-open-folder").unwrap();
                    assert!(location.origin.y >= name.bottom());
                    assert!(location.size.width >= px(300.));
                    assert!(folder.right() <= px(464.));
                    clicked.set(0);
                    cx.simulate_click(folder.center(), Modifiers::default());
                    assert_eq!(clicked.get(), 4, "Folder preview must not confirm deletion");
                }
                for (id, expected) in [
                    ("delete-cancel", 1),
                    ("delete-keep", 2),
                    ("delete-files", 3),
                ] {
                    let bounds = cx.debug_bounds(id).unwrap();
                    assert!(
                        bounds.origin.y >= px(34.),
                        "{id} overlaps the draggable header"
                    );
                    assert!(
                        bounds.bottom() <= px(height - 15.),
                        "{id} is clipped: {bounds:?}"
                    );
                    assert!(
                        bounds.right() <= px(464.),
                        "{id} extends past the right inset"
                    );
                    assert_eq!(bounds.size.height, px(32.));
                    clicked.set(0);
                    cx.simulate_click(bounds.center(), Modifiers::default());
                    assert_eq!(clicked.get(), expected, "{id} did not receive the click");
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            let close = cx.debug_bounds("close").unwrap();
            let handle = cx.update(|window, _| window.window_handle());
            clicked.set(0);
            cx.simulate_click(close.center(), Modifiers::default());
            cx.cx.update(|cx| {
                assert!(handle.update(cx, |_, _, _| ()).is_err());
            });
            assert_eq!(clicked.get(), 0);
        }
    }
}
