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
mod motion;
mod notice;
mod peer_metrics;
mod preferences;
mod progress;
mod protocol_auth;
mod redownload;
mod selection;
mod sidebar;
#[cfg(windows)]
mod startup;
#[cfg(target_os = "macos")]
#[path = "startup_macos.rs"]
mod startup;
#[cfg(target_os = "linux")]
#[path = "startup_linux.rs"]
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
    #[cfg(target_os = "macos")]
    updates::macos::run_installer();
    let data_dir = ndm_core::application_data_directory();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Ok(executable) = std::env::current_exe() {
        let host = executable.with_file_name("ndm-host");
        if let Err(error) = ndm_host::browser::register_browser_hosts(&host, &[]) {
            eprintln!("Could not register browser integration: {error}");
        }
    }
    let background = std::env::args_os().any(|arg| arg == "--background");
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    let handoff = std::env::args_os().any(|arg| arg == "--handoff");
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    let show_existing = !background && !handoff;
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    let mut instance = match instance::Instance::acquire(&data_dir, show_existing) {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
        Err(error) => panic!("create instance lock: {error}"),
    };
    let preferences_path = data_dir.join("preferences.json");
    let mut preferences = match Preferences::load(&preferences_path) {
        Ok(preferences) => preferences,
        Err(_) => {
            #[cfg(target_os = "linux")]
            eprintln!("Could not read NDM Next settings or decrypt stored credentials.");
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
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        let result = (|| -> std::io::Result<()> {
            let current = startup::enabled()?;
            if let Some(desired) = preferences.start_at_login {
                if desired != current {
                    startup::set(desired)?;
                }
            } else {
                preferences.start_at_login = Some(current);
            }
            preferences.save(&preferences_path)
        })();
        if let Err(error) = result {
            eprintln!("Could not initialize startup preference: {error}");
        }
    }
    let store = Arc::new(Store::open(&data_dir.join("tasks.db")).expect("open task database"));
    if let Ok(Some(limits)) = store.download_limits() {
        preferences.max_active_tasks = limits.active_tasks as usize;
        preferences.max_connections = limits.download_threads as usize;
    }
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
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    let (ui_sender, ui_commands) = tokio::sync::mpsc::unbounded_channel();
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        let scheduler = scheduler.clone();
        let sender = ui_sender.clone();
        instance.on_wake(move || {
            scheduler.refresh_download_limits();
            let _ = sender.send(instance::Command::Refresh);
        });
        let sender = ui_sender.clone();
        instance.on_show(move || {
            let _ = sender.send(instance::Command::Show);
        });
        let sender = ui_sender.clone();
        instance.on_shutdown(move || {
            let _ = sender.send(instance::Command::Quit);
        });
        #[cfg(not(target_os = "linux"))]
        instance.mark_ready().expect("signal desktop readiness");
    }
    #[cfg(target_os = "linux")]
    let instance = std::rc::Rc::new(instance);
    #[cfg(target_os = "linux")]
    let ready_instance = instance.clone();
    let application = gpui_kit::application().with_assets(application_assets());
    #[cfg(target_os = "macos")]
    application.on_reopen(|cx| {
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, cx| {
                if let Some(view) = Desktop::from_window(window, cx) {
                    view.update(cx, |this, cx| this.restore_frontend(cx));
                }
                chrome::activate_window(window, cx);
            });
        }
    });
    application.run(move |cx| {
        gpui_kit::init(cx);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
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
        // Keep browser-triggered launches in the background. Linux GPUI maps
        // the main window during creation, then the view minimizes it.
        #[cfg(target_os = "macos")]
        let show_main = !handoff;
        #[cfg(target_os = "linux")]
        let show_main = !background && !handoff;
        #[cfg(windows)]
        let show_main = !(handoff && background);
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        let show_main = true;
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(880.), px(400.))),
                show: show_main || cfg!(target_os = "linux"),
                focus: show_main,
                ..chrome::window_options()
            },
            cx,
            |window, cx| {
                chrome::configure_window_chrome(window);
                cx.new(|cx| {
                    let desktop = Desktop::new(
                        view::DesktopServices {
                            store,
                            scheduler,
                            runtime,
                            preferences,
                            preferences_path,
                            prepared_at_launch,
                            #[cfg(any(windows, target_os = "macos"))]
                            ui_sender,
                            #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
                            commands: ui_commands,
                        },
                        window,
                        cx,
                    );
                    desktop.refresh_managed_skill(cx);
                    desktop
                })
            },
        )
        .expect("open NDM Next");
        // On Linux, GPUI connects to the display during run. A browser
        // handoff must wait until the desktop has opened successfully.
        #[cfg(target_os = "linux")]
        ready_instance
            .mark_ready()
            .expect("signal desktop readiness");
        if show_main && !background {
            cx.activate(true);
        }
        #[cfg(target_os = "macos")]
        updates::macos::finish_update();
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
            IconName::Store,
            IconName::Sparkles,
            IconName::Terminal,
            IconName::Film,
            IconName::Network,
            IconName::Puzzle,
            IconName::SearchX,
            IconName::PackageOpen,
            IconName::ArrowDownToLine,
            IconName::ArrowDown,
            IconName::CircleCheck,
            IconName::Settings2,
            IconName::PanelLeftClose,
            IconName::PanelLeftOpen,
            IconName::FolderOpen,
            IconName::Plus,
            IconName::ChevronDown,
            IconName::ChevronUp,
            IconName::WindowMinimize,
            IconName::WindowMaximize,
            IconName::WindowRestore,
            IconName::WindowClose,
            IconName::ChartPie,
            IconName::Close,
            IconName::Delete,
            IconName::ExternalLink,
            IconName::File,
            IconName::Pause,
            IconName::Play,
            IconName::Zap,
            IconName::Clock,
            IconName::Trash,
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
