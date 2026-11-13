#![cfg_attr(windows, windows_subsystem = "windows")]

mod instance;
mod preferences;
mod view;

use gpui_kit::{
    component::{Theme as ComponentTheme, ThemeMode},
    prelude::*,
    *,
};
use ndm_core::{Scheduler, SchedulerOptions, Store};
use preferences::{Preferences, Theme};
use std::{path::PathBuf, sync::Arc};
use view::Desktop;

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
    let data_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("NDM Next");
    #[cfg(windows)]
    let mut instance = match instance::Instance::acquire(&data_dir).expect("create instance lock") {
        Some(instance) => instance,
        None => return,
    };
    let preferences_path = data_dir.join("preferences.json");
    let preferences = Preferences::load(&preferences_path).unwrap_or_default();
    let store = Arc::new(Store::open(&data_dir.join("tasks.db")).expect("open task database"));
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("start runtime"),
    );
    let scheduler = {
        let _guard = runtime.enter();
        Scheduler::start(
            store.clone(),
            SchedulerOptions {
                max_active_tasks: preferences.max_active_tasks,
                http: ndm_core::HttpOptions {
                    max_connections: preferences.max_connections,
                    ..Default::default()
                },
                auto_resume: preferences.auto_resume,
            },
        )
        .expect("start scheduler")
    };
    #[cfg(windows)]
    {
        let scheduler = scheduler.clone();
        instance.on_wake(move || scheduler.notify());
    }
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        apply_theme(preferences.theme, cx);
        let bounds = Bounds::centered(None, size(px(1120.), px(720.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    Desktop::new(
                        store,
                        scheduler,
                        runtime,
                        preferences,
                        preferences_path,
                        window,
                        cx,
                    )
                })
            },
        )
        .expect("open NDM Next");
        cx.activate(true);
    });
}
