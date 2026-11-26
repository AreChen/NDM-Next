use super::*;
use crate::{
    palette::{CustomPalette, DownloadStatus},
    preferences::ColorScheme,
};
use ndm_core::SettingsPatch;
use serde_json::{Value, json};

impl Desktop {
    fn settings_snapshot(&self) -> Value {
        let p = &self.preferences;
        let hex = |color: u32| format!("#{color:06X}");
        let download_colors = ndm_core::DOWNLOAD_COLOR_KEYS
            .into_iter()
            .zip(DownloadStatus::ALL)
            .map(|(key, status)| {
                (
                    key.to_owned(),
                    json!(p.download_colors.get(status).map(hex)),
                )
            })
            .collect::<serde_json::Map<String, Value>>();
        let proxy_url = p
            .proxy
            .as_ref()
            .and_then(|proxy| reqwest::Url::parse(&proxy.url).ok())
            .and_then(|url| {
                Some(format!(
                    "{}://{}:{}",
                    url.scheme(),
                    url.host()?,
                    url.port_or_known_default()?
                ))
            });
        json!({
            "active_tasks":p.max_active_tasks,"download_threads":p.max_connections,
            "speed_limit_kib_s":p.max_kib_per_second,"download_directory":p.download_dir,"use_system_download_directory":p.use_system_download_dir,
            "cache_retention_days":p.cache_retention_days,"user_agent":p.user_agent,"effective_user_agent":p.user_agent.as_deref().unwrap_or(ndm_core::CHROME_USER_AGENT),"auto_resume":p.auto_resume,
            "auto_update":p.auto_update,"ui_animations":p.ui_animations,"download_route":p.download_route,
            "tun_direct_exit":p.tun_egress.as_ref().map(|e|e.input_value()),"use_saved_proxy":p.proxy_enabled_by_default,"saved_proxy_available":p.proxy.is_some(),
            "tun_direct_port":p.tun_egress.as_ref().and_then(|egress| match egress {
                ndm_core::TunEgress::LocalPort(port) => Some(port.get()), _ => None
            }),
            "proxy_url":proxy_url,"proxy_has_credentials":p.proxy.as_ref().is_some_and(|proxy|proxy.username.is_some()),
            "cache_directory":p.cache_dir,
            "sidebar_collapsed":p.sidebar_collapsed,"sidebar_width":p.sidebar_width,
            "start_at_login":self.startup_enabled,
            "theme":match p.theme { Theme::System=>"system",Theme::Light=>"light",Theme::Dark=>"dark" },
            "color_scheme":p.color_scheme.key(),
            "theme_colors":{"primary":hex(p.custom_palette.primary), "secondary":p.custom_palette.secondary.map(hex), "surface":p.custom_palette.surface.map(hex)},
            "download_colors":download_colors,
            "language":p.language.code()
        })
    }

    pub(super) fn process_ai_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (id, patch) in self.store.pending_settings().unwrap_or_default() {
            let result = if !self.store.ai_enabled().unwrap_or(false) {
                Err("disabled")
            } else if let Some(patch) = patch {
                self.apply_ai_settings(patch, window, cx)
            } else {
                Ok(())
            };
            let reply = match result {
                Ok(()) => self.settings_snapshot(),
                Err(code) => json!({"error":code}),
            };
            let _ = self.store.answer_settings(id, &reply);
        }
    }

    fn apply_ai_settings(
        &mut self,
        patch: SettingsPatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        patch.validate()?;
        if self.storage_busy
            && (patch.download_directory.is_some()
                || patch.cache_directory.is_some()
                || patch.cache_retention_days.is_some())
        {
            return Err("busy");
        }
        let old = &self.preferences;
        let mut p = old.clone();
        if let Some(value) = patch.speed_limit_kib_s {
            p.max_kib_per_second = value;
        }
        if let Some(value) = patch.user_agent {
            p.user_agent = (!value.is_empty()).then_some(value);
        }
        if let Some(value) = patch.auto_resume {
            p.auto_resume = value;
        }
        if let Some(value) = patch.auto_update {
            p.auto_update = value;
        }
        if let Some(value) = patch.ui_animations {
            p.ui_animations = value;
        }
        if let Some(value) = patch.cache_retention_days {
            p.cache_retention_days = value;
            p.last_cache_cleanup = None;
        }
        if let Some(value) = patch.download_directory {
            let system = ndm_core::system_download_directory();
            p.download_dir = if value.is_empty() {
                system.clone()
            } else {
                value.into()
            };
            p.use_system_download_dir = p.download_dir == system;
        }
        if let Some(value) = patch.theme {
            p.theme = match value.as_str() {
                "light" => Theme::Light,
                "dark" => Theme::Dark,
                _ => Theme::System,
            };
        }
        if let Some(value) = patch.cache_directory {
            let directory = if value.is_empty() {
                ndm_core::default_cache_directory()
            } else {
                value.into()
            };
            crate::storage::set_cache_directory(&mut p, directory);
        }
        if let Some(value) = patch.start_at_login {
            p.start_at_login = Some(value);
        }
        if let Some(value) = patch.sidebar_collapsed {
            p.sidebar_collapsed = value;
        }
        if let Some(value) = patch.sidebar_width {
            p.sidebar_width = value;
        }
        if let Some(colors) = patch.theme_colors {
            for (role, value) in colors {
                let color = value.as_deref().map(ndm_core::parse_rgb).transpose()?;
                match role.as_str() {
                    "primary" => {
                        p.custom_palette.primary = color.unwrap_or(CustomPalette::default().primary)
                    }
                    "secondary" => p.custom_palette.secondary = color,
                    "surface" => {
                        p.custom_palette.surface = color;
                        p.custom_palette.legacy_surfaces = false;
                    }
                    _ => unreachable!("validated theme color role"),
                }
            }
            p.color_scheme = ColorScheme::Custom;
        }
        if let Some(value) = patch.color_scheme {
            p.color_scheme = ColorScheme::from_key(&value).ok_or("color_scheme")?;
        }
        if let Some(colors) = patch.download_colors {
            for (key, status) in ndm_core::DOWNLOAD_COLOR_KEYS
                .into_iter()
                .zip(DownloadStatus::ALL)
            {
                if let Some(value) = colors.get(key) {
                    p.download_colors.set(
                        status,
                        value.as_deref().map(ndm_core::parse_rgb).transpose()?,
                    );
                }
            }
        }
        if let Some(value) = patch.language {
            p.language = Language::from_code(&value).ok_or("language")?;
        }
        if let Some(value) = patch.download_route {
            p.download_route = value;
        }
        if let Some(value) = patch.tun_direct_exit {
            p.tun_egress = if value.is_empty() {
                None
            } else {
                Some(ndm_core::TunEgress::parse(&value).map_err(|_| "tun")?)
            };
        }
        if let Some(port) = patch.tun_direct_port {
            p.tun_egress = std::num::NonZeroU16::new(port).map(ndm_core::TunEgress::LocalPort);
        }
        if let Some(proxy) = patch.proxy {
            let clearing = proxy.url.is_empty();
            p.proxy = (!clearing).then_some(proxy);
            if clearing {
                p.proxy_enabled_by_default = false;
            }
        }
        if let Some(value) = patch.use_saved_proxy {
            p.proxy_enabled_by_default = value;
        }
        if p.proxy_enabled_by_default && p.proxy.is_none() {
            return Err("proxy");
        }
        if p.download_route == ProxyMode::TunDirect && p.tun_egress.is_none() {
            return Err("tun");
        }
        #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
        let startup = patch
            .start_at_login
            .filter(|value| *value != self.startup_enabled);
        if &p == old && patch.start_at_login.is_none() {
            return Ok(());
        }
        #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
        if let Some(value) = startup {
            crate::startup::set(value).map_err(|_| "startup")?;
        }
        if p.save(&self.preferences_path).is_err() {
            #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
            if startup.is_some() {
                let _ = crate::startup::set(self.startup_enabled);
            }
            return Err("save");
        }
        #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
        if let Some(value) = startup {
            self.startup_enabled = value;
        }
        let old = std::mem::replace(&mut self.preferences, p);
        let p = &self.preferences;
        self.scheduler.update_options(p.scheduler_options());
        if old.ui_animations != p.ui_animations {
            cx.set_reduce_motion(!p.ui_animations);
        }
        if old.auto_resume != p.auto_resume {
            self.pending_auto_resume = p.auto_resume;
        }
        if old.proxy_enabled_by_default != p.proxy_enabled_by_default {
            self.pending_proxy_enabled = p.proxy_enabled_by_default;
        }
        if old.download_route != p.download_route {
            self.pending_route = p.download_route;
        }
        if (old.sidebar_collapsed, old.sidebar_width) != (p.sidebar_collapsed, p.sidebar_width) {
            self.sidebar =
                crate::sidebar::Sidebar::new(p.sidebar_collapsed, p.sidebar_width.into());
        }
        let mut set = |field: &Entity<InputState>, value: String| {
            field.update(cx, |input, cx| input.set_value(value, window, cx))
        };
        if old.max_kib_per_second != p.max_kib_per_second {
            set(&self.speed_input, p.max_kib_per_second.to_string());
        }
        if old.user_agent != p.user_agent {
            set(
                &self.user_agent_input,
                p.user_agent.clone().unwrap_or_default(),
            );
        }
        if old.cache_retention_days != p.cache_retention_days {
            set(&self.retention_input, p.cache_retention_days.to_string());
        }
        if old.tun_egress != p.tun_egress {
            set(
                &self.tun_egress_input,
                p.tun_egress
                    .as_ref()
                    .map(|e| e.input_value())
                    .unwrap_or_default(),
            );
        }
        if old.cache_dir != p.cache_dir {
            set(
                &self.cache_input,
                p.cache_dir.to_string_lossy().into_owned(),
            );
        }
        if old.proxy != p.proxy {
            set(
                &self.proxy_url_input,
                p.proxy
                    .as_ref()
                    .map(|proxy| proxy.url.clone())
                    .unwrap_or_default(),
            );
            set(
                &self.proxy_user_input,
                p.proxy
                    .as_ref()
                    .and_then(|proxy| proxy.username.clone())
                    .unwrap_or_default(),
            );
            set(
                &self.proxy_password_input,
                p.proxy
                    .as_ref()
                    .and_then(|proxy| proxy.password.clone())
                    .unwrap_or_default(),
            );
        }
        if old.download_dir != p.download_dir {
            if self.directory_input.read(cx).value().as_ref() == old.download_dir.to_string_lossy()
            {
                self.directory_input.update(cx, |input, cx| {
                    input.set_value(p.download_dir.to_string_lossy().into_owned(), window, cx)
                });
            }
            self.storage_download_input.update(cx, |input, cx| {
                input.set_value(p.download_dir.to_string_lossy().into_owned(), window, cx)
            });
        }
        let language_changed = old.language != p.language;
        let palette_changed = old.color_scheme != p.color_scheme
            || old.custom_palette != p.custom_palette
            || old.download_colors != p.download_colors;
        if palette_changed {
            self.color_editor = None;
        }
        if old.theme != p.theme || palette_changed {
            self.refresh_appearance(cx);
        }
        if language_changed {
            self.refresh_language(window, cx);
        }
        cx.notify();
        Ok(())
    }
}
