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
