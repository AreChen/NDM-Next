#![cfg_attr(windows, windows_subsystem = "windows")]

mod branding;
mod chrome;
mod confirm_delete;
mod deletion;
mod failure;
mod filenames;
mod files;
mod instance;
mod library;
mod notice;
mod preferences;
mod progress;
mod protocol_auth;
mod speed;
#[cfg(windows)]
mod startup;
#[cfg(target_os = "macos")]
#[path = "startup_macos.rs"]
mod startup;
mod storage;
#[cfg(any(windows, target_os = "macos"))]
mod tray;
mod updates;
mod view;

use gpui_kit::{
    component::{Theme as ComponentTheme, ThemeMode},
    prelude::*,
    *,
};
use ndm_core::{Scheduler, Store, TaskState};
use preferences::{Preferences, Theme};
use std::sync::Arc;
use view::Desktop;

fn application_assets() -> impl AssetSource {
    branding::Assets
}

pub(crate) fn apply_theme(theme: Theme, cx: &mut App) {
    let mode = match theme {
        Theme::Dark => ThemeMode::Dark,
        Theme::Light => ThemeMode::Light,
        Theme::System => {
            if matches!(
                cx.window_appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ) {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            }
        }
    };
    ComponentTheme::change(mode, None, cx);
}

fn main() {
    let data_dir = ndm_core::application_data_directory();
    #[cfg(target_os = "macos")]
    if let Ok(executable) = std::env::current_exe() {
        let host = executable.with_file_name("ndm-host");
        if let Err(error) = ndm_host::macos::register_browser_hosts(&host, &[]) {
            eprintln!("Could not register browser integration: {error}");
        }
    }
    #[cfg(any(windows, target_os = "macos"))]
    let show_existing = !std::env::args_os().any(|arg| arg == "--background" || arg == "--handoff");
    #[cfg(any(windows, target_os = "macos"))]
    let mut instance = match instance::Instance::acquire(&data_dir, show_existing) {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
        Err(error) => panic!("create instance lock: {error}"),
    };
    let preferences_path = data_dir.join("preferences.json");
    let preferences = match Preferences::load(&preferences_path) {
        Ok(preferences) => preferences,
        Err(_) => {
            #[cfg(windows)]
            {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    MB_ICONERROR, MB_OK, MessageBoxW,
                };
                let text: Vec<u16> = "无法读取设置或解密代理凭据。请检查当前 Windows 账户和设置文件。\nCould not read settings or decrypt proxy credentials. Check your Windows account and settings file.\0".encode_utf16().collect();
                let title: Vec<u16> = "NDM Next\0".encode_utf16().collect();
                // SAFETY: both buffers are null-terminated and remain alive during the call.
                unsafe {
                    MessageBoxW(
                        std::ptr::null_mut(),
                        text.as_ptr(),
                        title.as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            return;
        }
    };
    let store = Arc::new(Store::open(&data_dir.join("tasks.db")).expect("open task database"));
    let prepared_at_launch = store
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|task| task.state == TaskState::Prepared)
        .map(|task| task.id)
        .collect();
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("start runtime"),
    );
    let scheduler = {
        let _guard = runtime.enter();
        Scheduler::start(store.clone(), preferences.scheduler_options()).expect("start scheduler")
    };
    #[cfg(any(windows, target_os = "macos"))]
    let (ui_sender, ui_commands) = tokio::sync::mpsc::unbounded_channel();
    #[cfg(any(windows, target_os = "macos"))]
    {
        let scheduler = scheduler.clone();
        let sender = ui_sender.clone();
        instance.on_wake(move || {
            scheduler.notify();
            let _ = sender.send(tray::Command::Refresh);
        });
        let sender = ui_sender.clone();
        instance.on_show(move || {
            let _ = sender.send(tray::Command::Show);
        });
        let sender = ui_sender.clone();
        instance.on_shutdown(move || {
            let _ = sender.send(tray::Command::Quit);
        });
        instance.mark_ready().expect("signal desktop readiness");
    }
    let application = gpui_kit::application().with_assets(application_assets());
    #[cfg(target_os = "macos")]
    application.on_reopen(|cx| {
        cx.activate(true);
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
    });
    application.run(move |cx| {
        gpui_kit::init(cx);
        #[cfg(target_os = "macos")]
        {
            let runtime = runtime.clone();
            let scheduler = scheduler.clone();
            cx.on_app_quit(move |_| {
                let scheduler = scheduler.clone();
                let runtime = runtime.clone();
                async move {
                    let _ = runtime
                        .spawn(async move { scheduler.shutdown().await })
                        .await;
                }
            })
            .detach();
        }
        apply_theme(preferences.theme, cx);
        let bounds = Bounds::centered(None, size(px(1000.), px(500.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(880.), px(400.))),
                ..gpui_kit::component::TitleBar::window_options()
            },
            cx,
            |window, cx| {
                chrome::round_window(window);
                cx.new(|cx| {
                    Desktop::new(
                        view::DesktopServices {
                            store,
                            scheduler,
                            runtime,
                            preferences,
                            preferences_path,
                            prepared_at_launch,
                            #[cfg(any(windows, target_os = "macos"))]
                            ui_sender,
                            #[cfg(any(windows, target_os = "macos"))]
                            commands: ui_commands,
                        },
                        window,
                        cx,
                    )
                })
            },
        )
        .expect("open NDM Next");
        if !std::env::args_os().any(|argument| argument == "--background") {
            cx.activate(true);
        }
    });
}

#[cfg(test)]
mod asset_tests {
    use super::application_assets;
    use gpui_kit::{AssetSource, assets::IconName};

    #[test]
    fn application_embeds_the_project_logo_as_a_png_asset() {
        let assets = application_assets();
        let logo = assets
            .load("branding/ndm-next.png")
            .unwrap()
            .expect("project logo");
        assert_eq!(&logo[..8], b"\x89PNG\r\n\x1a\n");
        assert!(
            assets
                .list("branding/")
                .unwrap()
                .iter()
                .any(|path| path.as_ref() == "branding/ndm-next.png")
        );
    }

    #[test]
    fn application_provides_icons_for_navigation_and_window_controls() {
        let assets = application_assets();
        for icon in [
            IconName::LayoutDashboard,
            IconName::ArrowDown,
            IconName::CircleCheck,
            IconName::Settings2,
            IconName::PanelLeftClose,
            IconName::PanelLeftOpen,
            IconName::FolderOpen,
            IconName::Plus,
            IconName::ChevronDown,
            IconName::ChevronUp,
            IconName::ChartPie,
            IconName::Close,
            IconName::Delete,
            IconName::ExternalLink,
            IconName::File,
            IconName::Pause,
            IconName::Play,
            IconName::Zap,
            IconName::Square,
            IconName::WindowMinimize,
            IconName::WindowMaximize,
            IconName::WindowRestore,
            IconName::WindowClose,
        ] {
            let path = icon.path();
            let svg = assets.load(&path).expect("icon lookup must succeed");
            assert!(svg.is_some_and(|bytes| !bytes.is_empty()), "missing {path}");
        }
    }
}
