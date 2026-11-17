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
        .debug_selector(|| "download-search".into())
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

pub(crate) const TOOLBAR_HEIGHT: f32 = 48.;

pub(crate) fn window_options(toolbar: bool) -> WindowOptions {
    let mut options = TitleBar::window_options();
    if toolbar && let Some(titlebar) = &mut options.titlebar {
        // Keep AppKit's traffic lights centered in the integrated toolbar.
        titlebar.traffic_light_position = Some(point(px(12.), px(16.)));
    }
    options
}

pub(crate) fn toolbar(dark: bool) -> TitleBar {
    TitleBar::new()
        .h(px(TOOLBAR_HEIGHT))
        .bg(rgb(if dark { 0x111625 } else { 0xf8f9fd }))
        .border_color(rgb(if dark { 0x263247 } else { 0xe5e9f2 }))
        // Native macOS controls are on the left; there is no right control group
        // to separate the last title action from the window edge.
        .when(cfg!(target_os = "macos"), |bar| bar.pr_3())
}

pub(crate) fn main_toolbar(
    title: &'static str,
    collapse: gpui_kit::component::button::Button,
    controls: impl IntoElement,
    dark: bool,
) -> TitleBar {
    toolbar(dark)
        .child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap_2()
                .pr_3()
                .child(crate::branding::logo(px(24.)))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child("NDM Next"),
                )
                .child(title_bar_action("sidebar-title-action", collapse))
                .child(
                    div()
                        .w(px(108.))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title),
                ),
        )
        .child(title_bar_action("toolbar-actions", controls).flex_1())
}

pub(crate) fn floating_header(title: impl IntoElement, dark: bool) -> TitleBar {
    TitleBar::new()
        .h(px(34.))
        .bg(rgba(0))
        .border_b_0()
        .when(cfg!(target_os = "macos"), |bar| bar.pr_3())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(rgb(if dark { 0xaebbd1 } else { 0x61718a }))
                .child(crate::branding::logo(px(18.)))
                .child(title),
        )
}

pub(crate) fn floating_surface(dark: bool) -> Background {
    // One static gradient, shared underneath header and body; no extra frames.
    linear_gradient(
        110.,
        linear_color_stop(rgb(if dark { 0x151a2e } else { 0xf8f7ff }), 0.),
        linear_color_stop(rgb(if dark { 0x101d27 } else { 0xf2f8fb }), 1.),
    )
}

/// Match the *client* area, not an assumed outer frame. A fixed-size modal
/// has different native insets from a resizable window, especially across DPI.
#[cfg(windows)]
pub(crate) fn fit_content(window: &mut Window, content: Size<Pixels>, cx: &App) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{
            GetClientRect, GetWindowRect, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos,
        },
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as _;
    let physical = content.to_device_pixels(window.scale_factor());
    let handle = Window::window_handle(window);
    let mut app = cx.to_async();
    // SetWindowPos synchronously delivers WM_SIZE. Run outside the current App /
    // Window borrow, as GPUI's native resize does, or its callback can reenter us.
    cx.foreground_executor()
        .spawn(async move {
            if handle.update(&mut app, |_, _, _| ()).is_err() {
                return;
            }
            let mut outer = RECT::default();
            let mut client = RECT::default();
            // SAFETY: the live window owns hwnd; the output rectangles remain valid.
            unsafe {
                if GetWindowRect(hwnd, &mut outer) == 0 || GetClientRect(hwnd, &mut client) == 0 {
                    return;
                }
                if client.right - client.left == physical.width.0
                    && client.bottom - client.top == physical.height.0
                {
                    return;
                }
                let width =
                    physical.width.0 + (outer.right - outer.left) - (client.right - client.left);
                let height =
                    physical.height.0 + (outer.bottom - outer.top) - (client.bottom - client.top);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
                );
            }
        })
        .detach();
}

#[cfg(not(windows))]
pub(crate) fn fit_content(window: &mut Window, content: Size<Pixels>, _: &App) {
    window.resize(content);
}

pub(crate) fn title_bar_action(
    id: impl Into<ElementId>,
    action: impl IntoElement,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h_full()
        .min_w_0()
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
        AppContext, Context, Focusable, Hitbox, HitboxBehavior, InteractiveElement, IntoElement,
        Modifiers, ParentElement, Render, StatefulInteractiveElement, Styled, Window,
        WindowControlArea, canvas, div, point, px,
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

    struct ToolbarHarness {
        input: gpui::Entity<gpui::component::input::InputState>,
        focus: gpui::FocusHandle,
        clicks: Rc<Cell<usize>>,
    }

    impl Render for ToolbarHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicked = self.clicks.clone();
            let mut controls = div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .pr_3()
                .child(super::download_search(&self.input, &self.focus, "Search"));
            for (id, label) in [
                ("type", "Applications"),
                ("sort", "↓"),
                ("new", "+"),
                ("update", "99% · 9.9 MB/s"),
            ] {
                let clicked = self.clicks.clone();
                controls = controls.child(
                    Button::new(id)
                        .small()
                        .label(label)
                        .debug_selector(move || id.into())
                        .on_click(move |_, _, _| clicked.set(clicked.get() + 1)),
                );
            }
            div()
                .size_full()
                .track_focus(&self.focus)
                .child(super::main_toolbar(
                    "All downloads",
                    super::icon_button(
                        "collapse",
                        gpui::component::IconName::PanelLeftClose,
                        "Collapse sidebar",
                    )
                    .debug_selector(|| "collapse".into())
                    .on_click(move |_, _, _| clicked.set(clicked.get() + 1)),
                    controls,
                    true,
                ))
        }
    }

    #[gpui::test]
    fn integrated_toolbar_keeps_search_and_actions_accessible_at_minimum_width(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui::init);
        let clicks = Rc::new(Cell::new(0));
        let (view, cx) = cx.add_window_view({
            let clicks = clicks.clone();
            move |window, cx| ToolbarHarness {
                input: cx.new(|cx| {
                    gpui::component::input::InputState::new(window, cx).placeholder("Search")
                }),
                focus: super::page_focus(window, cx),
                clicks,
            }
        });
        for width in [880., 1000.] {
            cx.simulate_resize(gpui::size(px(width), px(500.)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            for id in ["collapse", "type", "sort", "new", "update"] {
                let bounds = cx.debug_bounds(id).unwrap();
                assert!(bounds.bottom() <= px(super::TOOLBAR_HEIGHT));
                assert!(
                    bounds.right() <= px(width - 12.),
                    "{id} is clipped: {bounds:?}"
                );
                let before = clicks.get();
                cx.simulate_click(bounds.center(), Modifiers::default());
                assert_eq!(clicks.get(), before + 1, "{id} cannot be clicked");
            }
            let search = cx.debug_bounds("download-search").unwrap();
            assert!(
                search.size.width >= px(140.),
                "Search collapsed: {search:?}"
            );
            cx.simulate_click(search.center(), Modifiers::default());
            cx.update(|window, cx| {
                assert!(
                    view.read(cx)
                        .input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            });
            cx.simulate_click(point(px(400.), px(120.)), Modifiers::default());
            cx.update(|window, cx| assert!(view.read(cx).focus.is_focused(window)));
        }
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
                    "test-title-action",
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
