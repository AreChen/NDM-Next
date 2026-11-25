use super::*;
use ndm_core::SettingsPatch;
use serde_json::{Value, json};

impl Desktop {
    fn settings_snapshot(&self) -> Value {
        let p = &self.preferences;
        json!({
            "active_tasks":p.max_active_tasks,"download_threads":p.max_connections,
            "speed_limit_kib_s":p.max_kib_per_second,"download_directory":p.download_dir,"use_system_download_directory":p.use_system_download_dir,
            "cache_retention_days":p.cache_retention_days,"user_agent":p.user_agent,"effective_user_agent":p.user_agent.as_deref().unwrap_or(ndm_core::CHROME_USER_AGENT),"auto_resume":p.auto_resume,
            "auto_update":p.auto_update,"ui_animations":p.ui_animations,"download_route":p.download_route,
            "tun_direct_exit":p.tun_egress.as_ref().map(|e|e.input_value()),"use_saved_proxy":p.proxy_enabled_by_default,"saved_proxy_available":p.proxy.is_some(),
            "theme":match p.theme { Theme::System=>"system",Theme::Light=>"light",Theme::Dark=>"dark" },
            "language":match p.language {Language::Chinese=>"zh-CN",Language::English=>"en"}
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
            && (patch.download_directory.is_some() || patch.cache_retention_days.is_some())
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
        if let Some(value) = patch.language {
            p.language = if value == "en" {
                Language::English
            } else {
                Language::Chinese
            };
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
        if let Some(value) = patch.use_saved_proxy {
            p.proxy_enabled_by_default = value;
        }
        if p.proxy_enabled_by_default && p.proxy.is_none() {
            return Err("proxy");
        }
        if p.download_route == ProxyMode::TunDirect && p.tun_egress.is_none() {
            return Err("tun");
        }
        if &p == old {
            return Ok(());
        }
        p.save(&self.preferences_path).map_err(|_| "save")?;
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
        if old.theme != p.theme {
            self.refresh_appearance(cx);
        }
        if language_changed {
            self.refresh_language(window, cx);
        }
        cx.notify();
        Ok(())
    }
}
