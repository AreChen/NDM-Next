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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub language: Language,
    pub theme: Theme,
    pub sidebar_collapsed: bool,
    pub download_dir: PathBuf,
    pub max_active_tasks: usize,
    pub max_connections: usize,
    pub auto_resume: bool,
    pub auto_update: bool,
    pub last_update_check: Option<u64>,
    pub available_update: Option<String>,
    pub max_kib_per_second: u64,
    pub proxy_enabled_by_default: bool,
    #[serde(skip)]
    pub proxy: Option<ndm_core::ProxyOptions>,
}

impl Default for Preferences {
    fn default() -> Self {
        let download_dir = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads");
        Self {
            language: Language::Chinese,
            theme: Theme::System,
            sidebar_collapsed: false,
            download_dir,
            max_active_tasks: 3,
            max_connections: 4,
            auto_resume: true,
            auto_update: true,
            last_update_check: None,
            available_update: None,
            max_kib_per_second: 0,
            proxy_enabled_by_default: false,
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
            auto_resume: self.auto_resume,
            http: ndm_core::HttpOptions {
                max_connections: self.max_connections,
                max_bytes_per_second: if self.max_kib_per_second == 0 {
                    None
                } else {
                    Some(self.max_kib_per_second.saturating_mul(1024))
                },
                proxy: self.default_proxy(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            language: Language::English,
            auto_update: false,
            last_update_check: Some(1_000),
            available_update: Some("99.0.0".into()),
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), preferences);
        fs::write(&path, br#"{"theme":"Light"}"#).unwrap();
        let restored = Preferences::load(&path).unwrap();
        assert_eq!(restored.theme, Theme::Light);
        assert_eq!(restored.max_connections, 4);
        assert!(restored.auto_update);
        assert_eq!(restored.last_update_check, None);
        assert_eq!(restored.available_update, None);
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
