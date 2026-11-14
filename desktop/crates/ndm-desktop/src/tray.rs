use crate::preferences::Language;
use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::sync::mpsc::UnboundedSender;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow},
};

#[derive(Clone, Copy)]
pub enum Command {
    Show,
    Refresh,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    open: MenuItem,
    quit: MenuItem,
}

impl Tray {
    pub fn create(
        sender: UnboundedSender<Command>,
        language: Language,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let open = MenuItem::with_id("ndm-open", "Open NDM Next", true, None);
        let quit = MenuItem::with_id("ndm-quit", "Quit", true, None);
        menu.append_items(&[&open, &quit])?;
        let image =
            image::load_from_memory(include_bytes!("../../../../extension/icons/icon-32.png"))?
                .into_rgba8();
        let (width, height) = image.dimensions();
        let icon = Icon::from_rgba(image.into_raw(), width, height)?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("NDM Next")
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
            "打开 NDM Next"
        } else {
            "Open NDM Next"
        });
        self.quit.set_text(if language == Language::Chinese {
            "退出"
        } else {
            "Quit"
        });
    }
}

fn hwnd(window: &Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

pub fn hide(window: &Window) {
    if let Some(handle) = hwnd(window) {
        // SAFETY: the handle belongs to this live GPUI window on its UI thread.
        unsafe {
            ShowWindow(handle, SW_HIDE);
        }
    }
}

pub fn show(window: &Window) {
    if let Some(handle) = hwnd(window) {
        // SAFETY: the handle belongs to this live GPUI window on its UI thread.
        unsafe {
            ShowWindow(handle, SW_SHOW);
            SetForegroundWindow(handle);
        }
        window.activate_window();
    }
}
