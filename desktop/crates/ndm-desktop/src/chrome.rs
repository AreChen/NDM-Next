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
    label: impl Into<SharedString>,
    selected: bool,
    dark: bool,
) -> gpui_kit::component::menu::PopupMenuItem {
    use gpui_kit::component::ActiveTheme;
    let label = label.into();
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
                let tone = cx
                    .try_global::<crate::palette::ThemeTone>()
                    .copied()
                    .unwrap_or_else(|| dark.into());
                row.bg(rgb(tone.rgb(0x302b50, 0xede9ff)))
                    .text_color(rgb(tone.rgb(0xd9d1ff, 0x523fa7)))
            })
            .child(label.clone())
    })
}

pub(crate) const TOOLBAR_HEIGHT: f32 = 48.;

pub(crate) fn window_options() -> WindowOptions {
    WindowOptions {
        #[cfg(target_os = "linux")]
        app_id: Some("com.ndmnext.desktop".into()),
        titlebar: Some(TitlebarOptions {
            #[cfg(target_os = "linux")]
            title: Some("OnesDM".into()),
            traffic_light_position: None,
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    }
}

pub(crate) fn main_toolbar(
    collapse: gpui_kit::component::button::Button,
    controls: impl IntoElement,
    tone: impl Into<crate::motion::ThemeTone>,
    sidebar_width: f32,
    zh: bool,
    minimize_on_close: bool,
) -> TitleBar {
    let tone = tone.into();
    let bar = TitleBar::new()
        .on_close_window(move |_, _window, cx| {
            #[cfg(any(windows, target_os = "macos"))]
            if minimize_on_close {
                crate::tray::park(_window, cx);
                return;
            }
            cx.quit();
        })
        .pl_3()
        .h(px(TOOLBAR_HEIGHT))
        .bg(rgb(tone.rgb(0x111625, 0xf8f9fd)))
        .border_color(rgb(tone.rgb(0x263247, 0xe5e9f2)))
        .child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                // Align search with the sidebar, resize handle and content inset.
                .w(px((sidebar_width + 20. - 12.).max(152.)))
                .gap_2()
                .pr_3()
                .child(crate::branding::logo(px(24.)))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child("OnesDM"),
                )
                .child(title_bar_action("sidebar-title-action", collapse)),
        )
        .child(controls);
    with_window_controls(bar, zh, true, minimize_on_close)
}

#[derive(Default)]
struct BlankDrag {
    hitbox: std::rc::Rc<std::cell::RefCell<Option<Hitbox>>>,
    origin: Option<Point<Pixels>>,
    #[cfg(test)]
    moves: usize,
}

/// Drag only when the background hitbox receives hover. Input/button/menu
/// hitboxes mask it; no native caption area extends beneath those controls.
pub(crate) fn blank_drag_region(
    id: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let state = window.use_keyed_state((id, 0usize), cx, |_, _| BlankDrag::default());
    blank_drag_surface(id, state, window, cx)
}

fn blank_drag_surface(
    id: &'static str,
    state: Entity<BlankDrag>,
    window: &mut Window,
    cx: &App,
) -> Stateful<Div> {
    let hitbox = state.read(cx).hitbox.clone();
    div()
        .id(id)
        .relative()
        .on_mouse_down(
            MouseButton::Left,
            window.listener_for(&state, |state, event: &MouseDownEvent, window, _| {
                state.origin = (event.click_count == 1
                    && !window.default_prevented()
                    && state
                        .hitbox
                        .borrow()
                        .as_ref()
                        .is_some_and(|hitbox| hitbox.is_hovered(window)))
                .then_some(event.position);
                if state.origin.is_some() {
                    window.prevent_default();
                }
            }),
        )
        .on_mouse_move(
            window.listener_for(&state, |state, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    state.origin = None;
                } else if let Some(origin) = state.origin
                    && ((event.position.x - origin.x).abs() >= px(4.)
                        || (event.position.y - origin.y).abs() >= px(4.))
                {
                    state.origin = None;
                    #[cfg(test)]
                    {
                        state.moves += 1;
                    }
                    start_window_move(window, cx);
                }
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            window.listener_for(&state, |state, _, _, _| state.origin = None),
        )
        .child(
            canvas(
                move |bounds, window, _| {
                    *hitbox.borrow_mut() =
                        Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
}

#[cfg(windows)]
fn start_window_move(window: &Window, cx: &App) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, VK_LBUTTON},
        WindowsAndMessaging::{HTCAPTION, PostMessageW, SC_MOVE, WM_SYSCOMMAND},
    };
    let Ok(raw) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(raw) = raw.as_raw() else {
        return;
    };
    let hwnd = raw.hwnd.get();
    let handle = Window::window_handle(window);
    let mut app = cx.to_async();
    // GPUI's start_window_move is a no-op on Windows. Queue the native move
    // outside the App/Window borrow; native capture changes can reenter GPUI.
    cx.foreground_executor()
        .spawn(async move {
            if handle.update(&mut app, |_, _, _| ()).is_err() {
                return;
            }
            // SAFETY: hwnd belongs to the live, user-dragged window. Posting
            // hands the move loop to Windows without holding a GPUI borrow.
            unsafe {
                if GetAsyncKeyState(VK_LBUTTON as i32) >= 0 {
                    return;
                }
                ReleaseCapture();
                PostMessageW(hwnd as _, WM_SYSCOMMAND, (SC_MOVE | HTCAPTION) as usize, 0);
            }
        })
        .detach();
}

#[cfg(all(not(windows), not(test)))]
fn start_window_move(window: &Window, _: &App) {
    window.start_window_move();
}

// GPUI's test window has no native move implementation. The drag harness
// records the request before reaching this boundary.
#[cfg(all(not(windows), test))]
fn start_window_move(_: &Window, _: &App) {}

pub(crate) fn floating_header(
    title: impl IntoElement,
    tone: impl Into<crate::motion::ThemeTone>,
    zh: bool,
    resizable: bool,
) -> TitleBar {
    let tone = tone.into();
    let bar = TitleBar::new()
        .pl_3()
        .h(px(34.))
        .bg(rgba(0))
        .border_b_0()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(rgb(tone.rgb(0xaebbd1, 0x61718a)))
                .child(crate::branding::logo(px(18.)))
                .child(title),
        );
    with_window_controls(bar, zh, resizable, false)
}

fn with_window_controls(
    bar: TitleBar,
    _zh: bool,
    _resizable: bool,
    _minimize_on_close: bool,
) -> TitleBar {
    // Kit renders the Windows caption buttons and their native hit targets.
    // On macOS, supply the same icons and layout in the shared title bar.
    #[cfg(target_os = "macos")]
    let bar = bar.child(MacWindowControls {
        zh: _zh,
        resizable: _resizable,
        minimize_on_close: _minimize_on_close,
    });
    bar
}

#[cfg(target_os = "macos")]
#[derive(IntoElement)]
struct MacWindowControls {
    zh: bool,
    resizable: bool,
    minimize_on_close: bool,
}

#[cfg(target_os = "macos")]
impl RenderOnce for MacWindowControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::{
            ActiveTheme, IconName,
            button::{ButtonCustomVariant, ButtonRounded, ButtonVariants},
        };

        let text = |zh, en| if self.zh { zh } else { en };
        let theme = cx.theme();
        let standard = ButtonCustomVariant::new(cx)
            .hover(theme.secondary_hover)
            .active(theme.secondary_active);
        let danger = ButtonCustomVariant::new(cx)
            .hover(theme.danger)
            .active(theme.danger_active);
        let control = |id: &'static str, icon, label| {
            icon_button(id, icon, label)
                .debug_selector(move || id.into())
                .rounded(ButtonRounded::None)
                .w(px(34.))
                .h_full()
        };
        let (zoom_icon, zoom_label) =
            if appkit_window(window).is_some_and(|window| window.isZoomed()) {
                (IconName::WindowRestore, text("还原", "Restore"))
            } else {
                (IconName::WindowMaximize, text("最大化", "Maximize"))
            };
        title_bar_action(
            "mac-window-controls",
            div()
                .flex()
                .h_full()
                .when(self.resizable, |row| {
                    row.child(
                        control(
                            "minimize",
                            IconName::WindowMinimize,
                            text("最小化", "Minimize"),
                        )
                        .custom(standard)
                        .on_click(|_, window, _| window.minimize_window()),
                    )
                    .child(
                        control("zoom", zoom_icon, zoom_label)
                            .custom(standard)
                            .on_click(|_, window, _| window.zoom_window()),
                    )
                })
                .child(
                    control(
                        "close",
                        IconName::WindowClose,
                        text("关闭窗口", "Close window"),
                    )
                    .custom(danger)
                    .on_click(move |_, window, cx| {
                        if self.minimize_on_close {
                            crate::tray::park(window, cx);
                        } else {
                            window.remove_window();
                        }
                    }),
                ),
        )
        .ml_auto()
        .flex_shrink_0()
    }
}

pub(crate) fn floating_surface(tone: impl Into<crate::motion::ThemeTone>) -> Background {
    let tone = tone.into();
    // One static gradient, shared underneath header and body; no extra frames.
    linear_gradient(
        110.,
        linear_color_stop(rgb(tone.rgb(0x151a2e, 0xf8f7ff)), 0.),
        linear_color_stop(rgb(tone.rgb(0x101d27, 0xf2f8fb)), 1.),
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
pub(crate) fn configure_window_chrome(window: &Window) {
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

#[cfg(target_os = "macos")]
pub(crate) fn configure_window_chrome(window: &Window) {
    use objc2_app_kit::NSWindowButton;

    if let Some(native) = appkit_window(window) {
        for kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = native.standardWindowButton(kind) {
                button.setHidden(true);
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn configure_window_chrome(_: &Window) {}

#[cfg(target_os = "macos")]
fn appkit_window(window: &Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(raw) = handle.as_raw() else {
        return None;
    };
    // SAFETY: GPUI exposes the live AppKit NSView on the main thread for the
    // lifetime of this borrowed handle. NSView::window returns a retained window.
    unsafe { raw.ns_view.cast::<NSView>().as_ref() }.window()
}

#[cfg(target_os = "macos")]
pub(crate) fn activate_window(window: &Window, cx: &mut App) {
    cx.activate(true);
    if let Some(native) = appkit_window(window) {
        let handle = window.window_handle();
        let executor = cx.foreground_executor().clone();
        let mut cx = cx.to_async();
        executor
            .spawn(async move {
                // AppKit can synchronously call GPUI's window delegates. Run
                // after the current update has released the app and window.
                if handle.update(&mut cx, |_, _, _| ()).is_err() {
                    return;
                }
                if native.isMiniaturized() {
                    native.deminiaturize(None);
                }
                let mtm = objc2::MainThreadMarker::from(&*native);
                // If macOS declines app activation from a background process,
                // put this window in front of the browser without taking focus.
                if !objc2_app_kit::NSApplication::sharedApplication(mtm).isActive() {
                    native.orderFrontRegardless();
                }
            })
            .detach();
    }
    window.activate_window();
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn activate_window(window: &Window, _: &mut App) {
    window.activate_window();
}

#[cfg(test)]
mod tests {
    use super::title_bar_action;
    use gpui::component::{Sizable, button::Button};
    use gpui::{
        AppContext, Context, Entity, Focusable, Hitbox, HitboxBehavior, InteractiveElement,
        IntoElement, Modifiers, MouseButton, ParentElement, Render, StatefulInteractiveElement,
        Styled, Window, WindowControlArea, canvas, div, point, px,
    };
    use gpui_kit as gpui;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    struct DragHarness {
        state: Entity<super::BlankDrag>,
        focus: gpui::FocusHandle,
        input: Entity<gpui::component::input::InputState>,
        clicks: Rc<Cell<usize>>,
    }

    impl Render for DragHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            super::blank_drag_surface("blank-drag", self.state.clone(), window, cx)
                .size_full()
                .track_focus(&self.focus)
                .flex()
                .flex_col()
                .p_4()
                .child(
                    div()
                        .id("scroll-content")
                        .flex_1()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            Button::new("action")
                                .small()
                                .w(px(100.))
                                .debug_selector(|| "action".into())
                                .label("Action")
                                .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                        )
                        .child(
                            div()
                                .id("drag-input")
                                .debug_selector(|| "drag-input".into())
                                .w(px(200.))
                                .child(gpui::component::input::Input::new(&self.input).small()),
                        )
                        .child(
                            div()
                                .id("selectable-row")
                                .block_mouse_except_scroll()
                                .h(px(32.))
                                .w(px(200.))
                                .debug_selector(|| "selectable-row".into())
                                .on_click(|_, _, _| {})
                                .child("Selectable download"),
                        )
                        .child(div().h(px(800.))),
                )
                .child(
                    gpui::uniform_list(
                        "virtual-tasks",
                        1,
                        cx.processor(|_, _, _, _| {
                            vec![
                                div()
                                    .id("virtual-task")
                                    .debug_selector(|| "virtual-task".into())
                                    .block_mouse_except_scroll()
                                    .w_full()
                                    .h(px(32.))
                                    .on_click(|_, _, _| {})
                                    .child("Download")
                                    .into_any_element(),
                            ]
                        }),
                    )
                    .h(px(120.))
                    .flex_shrink_0(),
                )
        }
    }

    #[gpui::test]
    fn blank_area_drag_does_not_capture_inputs_buttons_or_task_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui::init);
        let clicks = Rc::new(Cell::new(0));
        let (view, cx) = cx.add_window_view({
            let clicks = clicks.clone();
            move |window, cx| DragHarness {
                state: cx.new(|_| super::BlankDrag::default()),
                focus: cx.focus_handle(),
                input: cx.new(|cx| gpui::component::input::InputState::new(window, cx)),
                clicks,
            }
        });
        cx.simulate_resize(gpui::size(px(500.), px(400.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for (id, expected) in [
            ("action", false),
            ("drag-input", false),
            ("selectable-row", false),
            ("virtual-task", false),
        ] {
            let point = cx.debug_bounds(id).unwrap().center();
            cx.simulate_mouse_move(point, None, Modifiers::default());
            cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
            cx.update(|_, cx| {
                assert_eq!(
                    view.read(cx).state.read(cx).origin.is_some(),
                    expected,
                    "{id}"
                )
            });
            cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        }
        assert_eq!(clicks.get(), 1);
        for (index, blank) in [point(px(250.), px(250.)), point(px(350.), px(365.))]
            .into_iter()
            .enumerate()
        {
            cx.simulate_mouse_move(blank, None, Modifiers::default());
            cx.simulate_mouse_down(blank, MouseButton::Left, Modifiers::default());
            cx.update(|_, cx| assert!(view.read(cx).state.read(cx).origin.is_some()));
            cx.simulate_mouse_move(
                blank + point(px(2.), px(1.)),
                Some(MouseButton::Left),
                Modifiers::default(),
            );
            cx.update(|_, cx| assert_eq!(view.read(cx).state.read(cx).moves, index));
            let moved = blank + point(px(8.), px(5.));
            cx.simulate_mouse_move(moved, Some(MouseButton::Left), Modifiers::default());
            cx.update(|_, cx| {
                assert_eq!(view.read(cx).state.read(cx).moves, index + 1);
                assert!(view.read(cx).state.read(cx).origin.is_none());
            });
            cx.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());
        }
    }

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
        sidebar_width: f32,
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
                    super::icon_button(
                        "collapse",
                        gpui::component::IconName::PanelLeftClose,
                        "Collapse sidebar",
                    )
                    .debug_selector(|| "collapse".into())
                    .on_click(move |_, _, _| clicked.set(clicked.get() + 1)),
                    controls,
                    true,
                    self.sidebar_width,
                    false,
                    false,
                ))
                .child(
                    div()
                        .ml(px(self.sidebar_width + 20.))
                        .debug_selector(|| "first-task-edge".into())
                        .h(px(52.)),
                )
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
                sidebar_width: 180.,
            }
        });
        for width in [880., 1000.] {
            cx.simulate_resize(gpui::size(px(width), px(500.)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let search = cx.debug_bounds("download-search").unwrap();
            let task = cx.debug_bounds("first-task-edge").unwrap();
            assert_eq!(
                search.left(),
                task.left(),
                "Toolbar and task list must share an edge"
            );
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
            #[cfg(target_os = "macos")]
            {
                let minimize = cx.debug_bounds("minimize").unwrap();
                let zoom = cx.debug_bounds("zoom").unwrap();
                let close = cx.debug_bounds("close").unwrap();
                assert!(cx.debug_bounds("update").unwrap().right() <= minimize.left());
                assert_eq!(minimize.right(), zoom.left());
                assert_eq!(zoom.right(), close.left());
                assert_eq!(close.right(), px(width));
            }
            for sidebar_width in [160., 240., 320.] {
                cx.update(|window, cx| {
                    view.update(cx, |view, cx| {
                        view.sidebar_width = sidebar_width;
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let search = cx.debug_bounds("download-search").unwrap();
                let task = cx.debug_bounds("first-task-edge").unwrap();
                assert_eq!(search.left(), task.left());
            }
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.sidebar_width = 180.;
                    cx.notify();
                })
            });
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
