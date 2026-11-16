use gpui_kit::{component::TitleBar, prelude::*, *};

pub(crate) fn page_focus(window: &mut Window, cx: &mut App) -> FocusHandle {
    let handle = cx.focus_handle();
    handle.focus(window, cx);
    handle
}

pub(crate) fn download_search(
    input: &Entity<gpui_kit::component::input::InputState>,
    page_focus: &FocusHandle,
    label: &'static str,
) -> impl IntoElement {
    use gpui_kit::component::{Icon, IconName, Sizable, input::Input};
    let state = input.clone();
    let page_focus = page_focus.clone();
    div()
        .id("search-container")
        .flex_1()
        .min_w_0()
        .on_mouse_down_out(move |_, window, cx| {
            if state.read(cx).focus_handle(cx).is_focused(window) {
                page_focus.focus(window, cx);
            }
        })
        .child(
            Input::new(input)
                .small()
                .id("search-downloads")
                .w_full()
                .prefix(Icon::new(IconName::Search).small())
                .aria_label(label),
        )
}

/// Shared by task rows and the compact progress window.
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    icon: impl Into<gpui_kit::component::Icon>,
    label: &'static str,
) -> gpui_kit::component::button::Button {
    use gpui_kit::component::{
        Sizable,
        button::{Button, ButtonVariants},
    };
    Button::new(id)
        .small()
        .ghost()
        .size(px(28.))
        .flex_shrink_0()
        .icon(Into::<gpui_kit::component::Icon>::into(icon))
        .tooltip(label)
        .accessibility_label(label)
}

pub(crate) fn task_actions(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .flex_shrink_0()
        .on_click(|_, _, cx| cx.stop_propagation())
}

pub(crate) fn copy_download_link(url: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(url.to_owned()));
}

pub(crate) fn copy_link_button(
    id: impl Into<ElementId>,
    url: String,
    label: &'static str,
) -> gpui_kit::component::button::Button {
    icon_button(
        id,
        gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Copy),
        label,
    )
    .on_click(move |_, _, cx| copy_download_link(&url, cx))
}

pub(crate) fn choice_item(
    label: &'static str,
    selected: bool,
    dark: bool,
) -> gpui_kit::component::menu::PopupMenuItem {
    use gpui_kit::component::ActiveTheme;
    gpui_kit::component::menu::PopupMenuItem::element(move |_, cx| {
        div()
            // PopupMenu already pads the outer, hoverable row by 0.5rem.
            // Extend this background over that same area, without double padding.
            .flex_1()
            .mx_neg_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius.min(px(8.)))
            .when(selected, |row| {
                row.bg(rgb(if dark { 0x302b50 } else { 0xede9ff }))
                    .text_color(rgb(if dark { 0xd9d1ff } else { 0x523fa7 }))
            })
            .child(label)
    })
}

pub(crate) fn title_bar(title: impl Into<SharedString>, dark: bool) -> TitleBar {
    TitleBar::new()
        .bg(rgb(if dark { 0x111827 } else { 0xf8f9fd }))
        .border_color(rgb(if dark { 0x263247 } else { 0xe5e9f2 }))
        // Native macOS controls are on the left; there is no right control group
        // to separate the last title action from the window edge.
        .when(cfg!(target_os = "macos"), |bar| bar.pr_3())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(rgb(if dark { 0xaebbd1 } else { 0x61718a }))
                .child(crate::branding::logo(px(18.)))
                .child(title.into()),
        )
}

pub(crate) fn title_bar_action(action: impl IntoElement) -> impl IntoElement {
    div()
        .id("title-bar-action")
        .flex()
        .items_center()
        .h_full()
        .flex_shrink_0()
        // Exclude the ancestor's native Drag hitbox, including its scroll IDs.
        // A normal button (or block_mouse_except_scroll) still becomes HTCAPTION.
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(action)
}

#[cfg(windows)]
pub(crate) fn round_window(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };

    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        let preference = DWMWCP_ROUND;
        // SAFETY: the HWND belongs to this live window and preference is a valid value.
        unsafe {
            DwmSetWindowAttribute(
                handle.hwnd.get() as _,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&preference as *const i32).cast(),
                std::mem::size_of_val(&preference) as u32,
            );
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn round_window(_: &Window) {}

#[cfg(test)]
mod tests {
    use super::title_bar_action;
    use gpui::component::{Sizable, button::Button};
    use gpui::{
        Context, Hitbox, HitboxBehavior, InteractiveElement, IntoElement, Modifiers, ParentElement,
        Render, StatefulInteractiveElement, Styled, Window, WindowControlArea, canvas, div, point,
        px,
    };
    use gpui_kit as gpui;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    struct ActionHarness {
        drag: Rc<RefCell<Option<Hitbox>>>,
        clicks: Rc<Cell<usize>>,
    }

    struct SearchHarness {
        input: gpui::Entity<gpui::component::input::InputState>,
        page_focus: gpui::FocusHandle,
    }

    #[gpui::test]
    fn copy_link_action_preserves_the_entire_download_url(cx: &mut gpui::TestAppContext) {
        const URL: &str = "https://example.test/%E4%B8%8B%E8%BD%BD.zip?token=test%2Fvalue&x=1#part";
        struct Harness;
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                super::copy_link_button("copy-link", URL.into(), "Copy download link")
            }
        }
        cx.update(gpui::init);
        let (_, cx) = cx.add_window_view(|_, _| Harness);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(point(px(14.), px(14.)), Modifiers::default());
        cx.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some(URL)
            )
        });
    }

    #[gpui::test]
    fn task_action_clicks_do_not_toggle_row_selection(cx: &mut gpui::TestAppContext) {
        struct Harness(Rc<Cell<usize>>, Rc<Cell<usize>>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let selections = self.0.clone();
                let actions = self.1.clone();
                div()
                    .id("row")
                    .flex()
                    .items_center()
                    .w(px(240.))
                    .h(px(52.))
                    .on_click(move |_, _, _| selections.set(selections.get() + 1))
                    .child(
                        super::task_actions("actions").child(
                            super::icon_button(
                                "folder",
                                gpui::component::IconName::FolderOpen,
                                "Open folder",
                            )
                            .on_click(move |_, _, _| actions.set(actions.get() + 1)),
                        ),
                    )
            }
        }
        cx.update(gpui::init);
        let selections = Rc::new(Cell::new(0));
        let actions = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let selections = selections.clone();
            let actions = actions.clone();
            move |_, _| Harness(selections, actions)
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(point(px(14.), px(14.)), Modifiers::default());
        assert_eq!(actions.get(), 1);
        assert_eq!(selections.get(), 0);
        cx.simulate_click(
            point(px(140.), px(14.)),
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(selections.get(), 1);
    }

    impl Render for SearchHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .track_focus(&self.page_focus)
                .flex()
                .flex_col()
                .w(px(400.))
                .h(px(120.))
                .child(super::download_search(
                    &self.input,
                    &self.page_focus,
                    "Search",
                ))
                .child(div().h(px(40.)).w_full())
        }
    }

    #[gpui::test]
    fn search_only_keeps_focus_until_the_user_clicks_outside(cx: &mut gpui::TestAppContext) {
        use gpui::{AppContext, Focusable};
        cx.update(gpui::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let input = cx.new(|cx| gpui::component::input::InputState::new(window, cx));
            let page_focus = super::page_focus(window, cx);
            SearchHarness { input, page_focus }
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(view.read(cx).page_focus.is_focused(window));
            assert!(
                !view
                    .read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        });
        cx.simulate_click(point(px(40.), px(14.)), Modifiers::default());
        cx.update(|window, cx| {
            assert!(
                view.read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        });
        cx.simulate_click(point(px(40.), px(110.)), Modifiers::default());
        cx.update(|window, cx| {
            assert!(view.read(cx).page_focus.is_focused(window));
        });
    }

    impl Render for ActionHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let drag = self.drag.clone();
            let clicks = self.clicks.clone();
            div()
                .relative()
                .flex()
                .items_center()
                .w(px(400.))
                .h(px(34.))
                .child(
                    canvas(
                        move |bounds, window, _| {
                            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                            *drag.borrow_mut() = Some(hitbox.clone());
                            hitbox
                        },
                        |_, hitbox, window, _| {
                            window.insert_window_control_hitbox(WindowControlArea::Drag, hitbox);
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .child(title_bar_action(
                    Button::new("update")
                        .small()
                        .label("Upgrade")
                        .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                ))
        }
    }

    #[gpui::test]
    fn title_action_excludes_native_drag_hit_test_but_background_remains_draggable(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui::init);
        let drag = Rc::new(RefCell::new(None));
        let clicks = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let drag = drag.clone();
            let clicks = clicks.clone();
            move |_, _| ActionHarness { drag, clicks }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_mouse_move(point(px(10.), px(10.)), None, Modifiers::default());
        cx.update(|window, _| {
            // Windows uses the full hit-test ID list, just like should_handle_scroll,
            // rather than only hover IDs. Blocking scroll-exempt hits is insufficient.
            assert!(
                !drag.borrow().as_ref().unwrap().should_handle_scroll(window),
                "Upgrade must not be classified as a native title-bar drag"
            );
        });
        cx.simulate_click(point(px(10.), px(10.)), Modifiers::default());
        assert_eq!(clicks.get(), 1);
        cx.simulate_mouse_move(point(px(300.), px(10.)), None, Modifiers::default());
        cx.update(|window, _| {
            assert!(drag.borrow().as_ref().unwrap().should_handle_scroll(window))
        });
    }
}
