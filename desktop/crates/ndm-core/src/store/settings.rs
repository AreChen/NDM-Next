use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SettingsPatch {
    pub speed_limit_kib_s: Option<u64>,
    pub user_agent: Option<String>,
    pub auto_resume: Option<bool>,
    pub auto_update: Option<bool>,
    pub ui_animations: Option<bool>,
    pub cache_retention_days: Option<u32>,
    pub download_directory: Option<String>,
    pub download_route: Option<crate::ProxyMode>,
    pub tun_direct_exit: Option<String>,
    pub use_saved_proxy: Option<bool>,
    pub theme: Option<String>,
    pub language: Option<String>,
}

pub const UI_LANGUAGES: [&str; 9] = [
    "en", "zh-CN", "zh-Hant", "ja", "ko", "es", "pt-BR", "fr", "de",
];

impl SettingsPatch {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        if self.speed_limit_kib_s.is_some_and(|n| n > 1_000_000) {
            return Err("Speed limit must be 0–1000000 KiB/s.");
        }
        if self.cache_retention_days.is_some_and(|n| n > 3650) {
            return Err("Cache retention must be 0–3650 days.");
        }
        if self
            .user_agent
            .as_ref()
            .is_some_and(|v| !v.is_empty() && crate::validate_user_agent(v).is_err())
        {
            return Err("Invalid User-Agent.");
        }
        if self.download_directory.as_ref().is_some_and(|v| {
            !v.is_empty() && (!Path::new(v).is_absolute() || v.chars().any(char::is_control))
        }) {
            return Err(
                "Use an absolute download directory or an empty string for the system folder.",
            );
        }
        if self
            .tun_direct_exit
            .as_ref()
            .is_some_and(|s| !s.is_empty() && crate::TunEgress::parse(s).is_err())
        {
            return Err("Invalid TUN direct exit for this platform.");
        }
        if self
            .theme
            .as_deref()
            .is_some_and(|s| !matches!(s, "system" | "light" | "dark"))
        {
            return Err("Unknown theme.");
        }
        if self
            .language
            .as_deref()
            .is_some_and(|s| !UI_LANGUAGES.contains(&s))
        {
            return Err("Unknown language.");
        }
        Ok(())
    }
}

// The desktop is the sole writer of preferences.json. Short-lived messages carry
// only allowlisted settings. The host acknowledges a change only after a reply.
impl Store {
    pub fn request_settings(&self, patch: Option<&SettingsPatch>) -> Result<i64> {
        let bytes = secrets::protect(&serde_json::to_vec(&patch)?)?;
        let conn = self.connection()?;
        conn.execute(
            "DELETE FROM settings_requests WHERE expires < unixepoch()",
            [],
        )?;
        conn.execute(
            "INSERT INTO settings_requests(patch,expires) VALUES(?1,unixepoch()+30)",
            [bytes],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn pending_settings(&self) -> Result<Vec<(i64, Option<SettingsPatch>)>> {
        let conn = self.connection()?;
        let mut stmt=conn.prepare("SELECT id,patch FROM settings_requests WHERE response IS NULL AND expires>=unixepoch() ORDER BY id")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(1)?;
            out.push((
                row.get(0)?,
                serde_json::from_slice(&secrets::unprotect(&bytes)?)?,
            ));
        }
        Ok(out)
    }

    pub fn answer_settings(&self, id: i64, reply: &Value) -> Result<()> {
        self.connection()?.execute(
            "UPDATE settings_requests SET response=?2 WHERE id=?1",
            params![id, secrets::protect(&serde_json::to_vec(reply)?)?],
        )?;
        Ok(())
    }

    pub fn settings_reply(&self, id: i64) -> Result<Option<Value>> {
        let bytes: Option<Vec<u8>> = self.connection()?.query_row(
            "SELECT response FROM settings_requests WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        bytes
            .map(|b| Ok(serde_json::from_slice(&secrets::unprotect(&b)?)?))
            .transpose()
    }

    pub fn forget_settings_request(&self, id: i64) -> Result<()> {
        self.connection()?
            .execute("DELETE FROM settings_requests WHERE id=?1", [id])?;
        Ok(())
    }
}
