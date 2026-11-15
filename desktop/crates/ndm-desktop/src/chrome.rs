use gpui_kit::{component::TitleBar, prelude::*, *};

pub(crate) fn title_bar(title: impl Into<SharedString>, dark: bool) -> TitleBar {
    TitleBar::new()
        .bg(rgb(if dark { 0x111827 } else { 0xf8f9fd }))
        .border_color(rgb(if dark { 0x263247 } else { 0xe5e9f2 }))
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
        Context, Hitbox, HitboxBehavior, IntoElement, Modifiers, ParentElement, Render, Styled,
        Window, WindowControlArea, canvas, div, point, px,
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
