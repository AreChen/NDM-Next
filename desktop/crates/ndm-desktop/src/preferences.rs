use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    Chinese,
    English,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::Chinese, Self::English];

    pub const fn native_name(self) -> &'static str {
        match self {
            Self::Chinese => "简体中文",
            Self::English => "English",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorScheme {
    #[default]
    Classic,
    Silver,
    Ocean,
    Forest,
    Sunset,
    Custom,
}

impl ColorScheme {
    pub const ALL: [Self; 6] = [
        Self::Classic,
        Self::Silver,
        Self::Ocean,
        Self::Forest,
        Self::Sunset,
        Self::Custom,
    ];

    pub fn label(self, language: Language) -> &'static str {
        let (zh, en) = match self {
            Self::Classic => ("经典紫", "Classic violet"),
            Self::Silver => ("银环黑白", "Silver ring"),
            Self::Ocean => ("海蓝", "Ocean"),
            Self::Forest => ("森林绿", "Forest"),
            Self::Sunset => ("落日珊瑚", "Sunset coral"),
            Self::Custom => ("自定义", "Custom"),
        };
        if language == Language::Chinese {
            zh
        } else {
            en
        }
    }

    pub fn accents(self, custom: [u32; 2]) -> [u32; 2] {
        match self {
            Self::Classic => [0x885dff, 0x37c9f5],
            Self::Silver => [0xc7c7c7, 0x909090],
            Self::Ocean => [0x367aed, 0x1ebbcf],
            Self::Forest => [0x23885c, 0x9bb548],
            Self::Sunset => [0xd95e74, 0xeaa64b],
            Self::Custom => custom,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub language: Language,
    pub theme: Theme,
    pub color_scheme: ColorScheme,
    pub custom_accents: [u32; 2],
    pub sidebar_collapsed: bool,
    pub sidebar_width: u16,
    pub ui_animations: bool,
    pub download_dir: PathBuf,
    pub use_system_download_dir: bool,
    pub cache_dir: PathBuf,
    pub previous_cache_dirs: Vec<PathBuf>,
    pub cache_retention_days: u32,
    pub last_cache_cleanup: Option<u64>,
    pub max_active_tasks: usize,
    pub max_connections: usize,
    // Missing in older profiles: preserve their existing OS startup setting.
    #[serde(default)]
    pub start_at_login: Option<bool>,
    pub auto_resume: bool,
    pub auto_update: bool,
    pub last_update_check: Option<u64>,
    pub available_update: Option<String>,
    pub max_kib_per_second: u64,
    pub user_agent: Option<String>,
    pub proxy_enabled_by_default: bool,
    pub download_route: ndm_core::ProxyMode,
    pub tun_egress: Option<ndm_core::TunEgress>,
    #[serde(skip)]
    pub proxy: Option<ndm_core::ProxyOptions>,
}

impl Default for Preferences {
    fn default() -> Self {
        let download_dir = ndm_core::system_download_directory();
        Self {
            language: Language::Chinese,
            theme: Theme::System,
            color_scheme: ColorScheme::Classic,
            custom_accents: [0x885dff, 0x37c9f5],
            sidebar_collapsed: false,
            sidebar_width: crate::sidebar::DEFAULT_WIDTH as u16,
            ui_animations: true,
            download_dir,
            use_system_download_dir: true,
            cache_dir: ndm_core::default_cache_directory(),
            previous_cache_dirs: Vec::new(),
            cache_retention_days: 7,
            last_cache_cleanup: None,
            max_active_tasks: 3,
            max_connections: 16,
            start_at_login: Some(true),
            auto_resume: true,
            auto_update: true,
            last_update_check: None,
            available_update: None,
            max_kib_per_second: 0,
            user_agent: None,
            proxy_enabled_by_default: false,
            download_route: Default::default(),
            tun_egress: None,
            proxy: None,
        }
    }
}
impl Preferences {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match fs::read(path) {
            Ok(data) => {
                let value: serde_json::Value =
                    serde_json::from_slice(&data).map_err(std::io::Error::other)?;
                let mut preferences: Self =
                    serde_json::from_value(value.clone()).map_err(std::io::Error::other)?;
                let (directory, automatic) = ndm_core::resolve_download_directory(
                    value
                        .get("download_dir")
                        .and_then(|value| value.as_str())
                        .map(Path::new),
                    value
                        .get("use_system_download_dir")
                        .and_then(|value| value.as_bool()),
                );
                preferences.custom_accents =
                    preferences.custom_accents.map(|value| value.min(0xffffff));
                preferences.download_dir = directory;
                preferences.sidebar_width =
                    crate::sidebar::valid_width(f32::from(preferences.sidebar_width)) as u16;
                preferences.use_system_download_dir = automatic;
                if !preferences.cache_dir.is_absolute() {
                    preferences.cache_dir = ndm_core::default_cache_directory();
                }
                preferences.cache_retention_days = preferences.cache_retention_days.min(3650);
                preferences.user_agent = preferences
                    .user_agent
                    .filter(|value| ndm_core::validate_user_agent(value).is_ok());
                if let Some(encrypted) = value.get("proxy_protected") {
                    let encrypted: Vec<u8> =
                        serde_json::from_value(encrypted.clone()).map_err(std::io::Error::other)?;
                    let clear =
                        ndm_core::unprotect_user_data(&encrypted).map_err(std::io::Error::other)?;
                    preferences.proxy =
                        Some(serde_json::from_slice(&clear).map_err(std::io::Error::other)?);
                }
                Ok(preferences)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut value = serde_json::to_value(self).map_err(std::io::Error::other)?;
        if let Some(proxy) = &self.proxy {
            let clear = serde_json::to_vec(proxy).map_err(std::io::Error::other)?;
            let encrypted = ndm_core::protect_user_data(&clear).map_err(std::io::Error::other)?;
            value.as_object_mut().unwrap().insert(
                "proxy_protected".into(),
                serde_json::to_value(encrypted).map_err(std::io::Error::other)?,
            );
        }
        let data = serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
        temporary.write_all(&data)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(std::io::Error::other)?;
        Ok(())
    }

    pub fn default_proxy(&self) -> Option<ndm_core::ProxyOptions> {
        self.proxy_enabled_by_default
            .then(|| self.proxy.clone())
            .flatten()
    }

    pub fn scheduler_options(&self) -> ndm_core::SchedulerOptions {
        ndm_core::SchedulerOptions {
            max_active_tasks: self.max_active_tasks,
            cache_directory: Some(self.cache_dir.join("transfers")),
            auto_resume: self.auto_resume,
            http: ndm_core::HttpOptions {
                max_connections: self.max_connections,
                max_bytes_per_second: if self.max_kib_per_second == 0 {
                    None
                } else {
                    Some(self.max_kib_per_second.saturating_mul(1024))
                },
                proxy: self.default_proxy(),
                proxy_mode: self.download_route,
                tun_egress: self.tun_egress.clone(),
                user_agent: self.user_agent.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sidebar_width_round_trips_defaults_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let mut preferences = Preferences {
            sidebar_width: 264,
            sidebar_collapsed: true,
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        let loaded = Preferences::load(&path).unwrap();
        assert_eq!(loaded.sidebar_width, 264);
        assert!(loaded.sidebar_collapsed);
        preferences.sidebar_width = 999;
        preferences.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap().sidebar_width, 320);
        let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        old.as_object_mut().unwrap().remove("sidebar_width");
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(Preferences::load(&path).unwrap().sidebar_width, 180);
    }
    #[cfg(windows)]
    #[test]
    fn saved_proxy_only_becomes_a_default_after_explicit_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let mut preferences = Preferences {
            proxy: Some(ndm_core::ProxyOptions {
                url: "http://127.0.0.1:18085".into(),
                username: None,
                password: None,
            }),
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        let loaded = Preferences::load(&path).unwrap();
        assert!(loaded.proxy.is_some());
        assert!(loaded.scheduler_options().http.proxy.is_none());
        preferences.proxy_enabled_by_default = true;
        preferences.save(&path).unwrap();
        assert_eq!(
            Preferences::load(&path)
                .unwrap()
                .scheduler_options()
                .http
                .proxy,
            preferences.proxy
        );
        // Preferences from versions before the opt-in toggle also start disabled.
        let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        old.as_object_mut()
            .unwrap()
            .remove("proxy_enabled_by_default");
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(Preferences::load(&path).unwrap().default_proxy().is_none());
    }
    #[test]
    fn preferences_round_trip_and_default_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let preferences = Preferences {
            theme: Theme::Dark,
            color_scheme: ColorScheme::Custom,
            custom_accents: [0x001144, 0xfd2034],
            language: Language::English,
            auto_update: false,
            last_update_check: Some(1_000),
            available_update: Some("99.0.0".into()),
            user_agent: Some("NDM test default".into()),
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), preferences);
        fs::write(&path, br#"{"theme":"Light"}"#).unwrap();
        let restored = Preferences::load(&path).unwrap();
        assert_eq!(restored.theme, Theme::Light);
        assert_eq!(restored.color_scheme, ColorScheme::Classic);
        assert_eq!(restored.user_agent, None);
        assert_eq!(restored.max_connections, 16);
        assert_eq!(restored.start_at_login, None);
        assert!(restored.auto_update);
        assert_eq!(restored.last_update_check, None);
        assert_eq!(restored.available_update, None);
    }

    #[test]
    fn fresh_profiles_enable_startup_and_preserve_explicit_choices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let mut preferences = Preferences::load(&path).unwrap();
        assert_eq!(preferences.start_at_login, Some(true));
        assert_eq!(preferences.scheduler_options().http.max_connections, 16);
        assert!(preferences.ui_animations);
        preferences.start_at_login = Some(false);
        preferences.max_connections = 4;
        preferences.ui_animations = false;
        preferences.save(&path).unwrap();
        let loaded = Preferences::load(&path).unwrap();
        assert_eq!(loaded.start_at_login, Some(false));
        assert_eq!(loaded.max_connections, 4);
        assert!(!loaded.ui_animations);
    }

    #[cfg(windows)]
    #[test]
    fn proxy_credentials_are_encrypted_at_rest_and_restored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let preferences = Preferences {
            proxy: Some(ndm_core::ProxyOptions {
                url: "http://127.0.0.1:8080".into(),
                username: Some("unique-proxy-user".into()),
                password: Some("unique-proxy-password".into()),
            }),
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("unique-proxy-user"));
        assert!(!raw.contains("unique-proxy-password"));
        assert_eq!(Preferences::load(&path).unwrap(), preferences);
    }
}
