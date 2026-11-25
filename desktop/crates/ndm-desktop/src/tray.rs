use crate::{instance::Command, preferences::Language};
use gpui_kit::{App, Window};
#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::sync::mpsc::UnboundedSender;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow},
};

pub struct Tray {
    _icon: TrayIcon,
    open: MenuItem,
    quit: MenuItem,
}

pub(crate) fn park(window: &mut Window, cx: &mut App) {
    if let Some(view) = crate::view::Desktop::from_window(window, cx) {
        view.update(cx, |this, cx| this.release_frontend(cx));
        hide_after_draw(window, cx);
    }
}

pub(crate) fn hide_after_draw(window: &Window, cx: &App) {
    let handle = window.window_handle();
    // Draw the empty foreground once, after returning the view's mutable lease,
    // so the hidden window lets go of its previous element tree and hitboxes.
    cx.spawn(async move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            if crate::view::Desktop::from_window(window, cx)
                .is_some_and(|view| view.read(cx).is_parked())
            {
                window.draw(cx).clear(cx);
                hide(window);
            }
        });
    })
    .detach();
}

impl Tray {
    pub fn create(
        sender: UnboundedSender<Command>,
        language: Language,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let open = MenuItem::with_id("ndm-open", "Open OnesDM", true, None);
        let quit = MenuItem::with_id("ndm-quit", "Quit", true, None);
        menu.append_items(&[&open, &quit])?;
        let image =
            image::load_from_memory(include_bytes!("../../../../extension/icons/icon-32.png"))?
                .into_rgba8();
        let (width, height) = image.dimensions();
        let icon = Icon::from_rgba(image.into_raw(), width, height)?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("OnesDM")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()?;
        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id.as_ref() {
                "ndm-open" => Some(Command::Show),
                "ndm-quit" => Some(Command::Quit),
                _ => None,
            };
            if let Some(command) = command {
                let _ = menu_sender.send(command);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = sender.send(Command::Show);
            }
        }));
        let tray = Self {
            _icon: icon,
            open,
            quit,
        };
        tray.set_language(language);
        Ok(tray)
    }

    pub fn set_language(&self, language: Language) {
        self.open.set_text(if language == Language::Chinese {
            "打开 OnesDM"
        } else {
            "Open OnesDM"
        });
        self.quit.set_text(if language == Language::Chinese {
            "退出"
        } else {
            "Quit"
        });
    }
}

#[cfg(windows)]
fn hwnd(window: &Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

#[cfg(windows)]
fn hide(window: &Window) {
    if let Some(handle) = hwnd(window) {
        // SAFETY: the handle belongs to this live GPUI window on its UI thread.
        unsafe {
            ShowWindow(handle, SW_HIDE);
        }
    }
}

#[cfg(windows)]
pub fn show(window: &Window, _: &mut gpui_kit::App) {
    if let Some(handle) = hwnd(window) {
        // SAFETY: the handle belongs to this live GPUI window on its UI thread.
        unsafe {
            ShowWindow(handle, SW_SHOW);
            SetForegroundWindow(handle);
        }
        window.activate_window();
    }
}

#[cfg(target_os = "macos")]
fn hide(window: &Window) {
    window.minimize_window();
}
#[cfg(target_os = "macos")]
pub fn show(window: &Window, cx: &mut gpui_kit::App) {
    crate::chrome::activate_window(window, cx);
}
