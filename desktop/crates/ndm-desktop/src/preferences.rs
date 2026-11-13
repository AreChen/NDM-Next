use serde::{Deserialize, Serialize};
use std::{
    fs,
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
    pub download_dir: PathBuf,
    pub max_active_tasks: usize,
    pub max_connections: usize,
    pub auto_resume: bool,
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
            download_dir,
            max_active_tasks: 3,
            max_connections: 4,
            auto_resume: true,
        }
    }
}
impl Preferences {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data).map_err(std::io::Error::other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        fs::write(path, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_round_trip_and_default_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let preferences = Preferences {
            theme: Theme::Dark,
            language: Language::English,
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), preferences);
        fs::write(&path, br#"{"theme":"Light"}"#).unwrap();
        let restored = Preferences::load(&path).unwrap();
        assert_eq!(restored.theme, Theme::Light);
        assert_eq!(restored.max_connections, 4);
    }
}
