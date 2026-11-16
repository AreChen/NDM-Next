use crate::{chrome, preferences::Language, view::Desktop};
use gpui_kit::{
    component::{
        Sizable, Theme as ComponentTheme, ThemeMode,
        button::{Button, ButtonVariants},
    },
    prelude::*,
    *,
};
use ndm_core::TaskId;

pub(crate) fn open(
    ids: Vec<TaskId>,
    name: Option<String>,
    language: Language,
    parent: WeakEntity<Desktop>,
    cx: &mut App,
) -> Result<AnyWindowHandle, ()> {
    let height = if name.is_some() { 156. } else { 132. };
    let bounds = Bounds::centered(None, size(px(480.), px(height)), cx);
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            kind: WindowKind::Dialog,
            is_resizable: false,
            is_minimizable: false,
            ..gpui_kit::component::TitleBar::window_options()
        },
        cx,
        move |window, cx| {
            chrome::round_window(window);
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
                    name,
                    language,
                    parent,
                }
            })
        },
    )
    .map_err(|_| ())?;
    Ok(handle)
}

struct ConfirmDelete {
    ids: Vec<TaskId>,
    name: Option<String>,
    language: Language,
    parent: WeakEntity<Desktop>,
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = matches!(ComponentTheme::global(cx).mode, ThemeMode::Dark);
        let fg = if dark { 0xf0f3ff } else { 0x19253b };
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
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(if dark { 0x101726 } else { 0xf8f9fd }))
            .text_color(rgb(fg))
            .child(chrome::title_bar("NDM Next", dark))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_4()
                    .py_3()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_shrink_0()
                            .min_w_0()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .when_some(self.name.clone(), |panel, name| {
                                panel.child(div().text_sm().truncate().child(name))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(28.))
                            .flex_shrink_0()
                            .gap_2()
                            .child(
                                Button::new("delete-cancel")
                                    .small()
                                    .label(self.text("取消", "Cancel"))
                                    .on_click(
                                        cx.listener(|_, _, window, _| window.remove_window()),
                                    ),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("delete-keep")
                                    .small()
                                    .label(self.text("仅删除记录", "Keep files"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("delete-files")
                                    .small()
                                    .danger()
                                    .label(self.text("删除文件和记录", "Delete files"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm(true, window, cx)
                                    })),
                            ),
                    ),
            )
    }
}
