use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "en", alias = "English")]
    English,
    #[default]
    #[serde(rename = "zh-CN", alias = "Chinese")]
    Chinese,
    #[serde(rename = "zh-Hant")]
    TraditionalChinese,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "ko")]
    Korean,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "pt-BR")]
    BrazilianPortuguese,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "de")]
    German,
}

impl Language {
    pub const ALL: [Self; 9] = [
        Self::English,
        Self::Chinese,
        Self::TraditionalChinese,
        Self::Japanese,
        Self::Korean,
        Self::Spanish,
        Self::BrazilianPortuguese,
        Self::French,
        Self::German,
    ];

    pub const fn code(self) -> &'static str {
        self.names().0
    }

    pub const fn native_name(self) -> &'static str {
        self.names().1
    }

    const fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::English => ("en", "English"),
            Self::Chinese => ("zh-CN", "简体中文"),
            Self::TraditionalChinese => ("zh-Hant", "繁體中文"),
            Self::Japanese => ("ja", "日本語"),
            Self::Korean => ("ko", "한국어"),
            Self::Spanish => ("es", "Español"),
            Self::BrazilianPortuguese => ("pt-BR", "Português (Brasil)"),
            Self::French => ("fr", "Français"),
            Self::German => ("de", "Deutsch"),
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|lang| lang.code() == code)
    }

    pub fn system_default() -> Self {
        sys_locale::get_locales()
            .find_map(|tag| Self::from_system_tag(&tag))
            .unwrap_or(Self::English)
    }

    fn from_system_tag(tag: &str) -> Option<Self> {
        let tag = tag
            .split(['.', '@'])
            .next()?
            .replace('_', "-")
            .to_ascii_lowercase();
        let mut parts = tag.split('-');
        match parts.next()? {
            "zh" => {
                let parts: Vec<_> = parts.collect();
                let traditional = !parts.contains(&"hans")
                    && parts
                        .iter()
                        .any(|part| matches!(*part, "hant" | "tw" | "hk" | "mo"));
                Some(if traditional {
                    Self::TraditionalChinese
                } else {
                    Self::Chinese
                })
            }
            "pt" => Some(Self::BrazilianPortuguese),
            base => Self::from_code(base),
        }
    }

    pub fn apply_to_controls(self) {
        gpui_kit::component::set_locale(if self == Self::TraditionalChinese {
            "zh-TW"
        } else {
            self.code()
        });
    }

    pub fn text<'a>(self, chinese: &'a str, english: &'a str) -> &'a str {
        match self {
            Self::Chinese if !chinese.is_empty() => chinese,
            Self::Chinese | Self::English => english,
            _ => lookup(self, english).unwrap_or(english),
        }
    }

    pub fn plugin_text(self, value: &ndm_core::plugins::Localized) -> &str {
        self.text(&value.zh, &value.en)
    }

    pub fn runtime_error(self, error: &ndm_core::plugins::RuntimeDependencyError) -> String {
        use ndm_core::plugins::RuntimeDependencyError::*;
        match error {
            Missing {
                consumer,
                provider,
                version,
                tool,
            } => self.format(
                "插件 {consumer} 需要 {provider} {version} 提供的 {tool}。",
                "{consumer} needs {tool} from {provider} {version}.",
                &[
                    ("consumer", consumer),
                    ("provider", provider),
                    ("version", &version.to_string()),
                    ("tool", tool),
                ],
            ),
            RequiredBy { provider, consumer } => self.format(
                "插件 {consumer} 依赖 {provider}。",
                "{consumer} requires {provider}.",
                &[("provider", provider), ("consumer", consumer)],
            ),
            Busy(provider) => self.format(
                "正在运行的任务仍在使用 {provider} 的工具。",
                "A running download is using tools from {provider}.",
                &[("provider", provider)],
            ),
            Cycle => self
                .text(
                    "插件运行工具存在循环依赖。",
                    "Plugin tools have a circular dependency.",
                )
                .to_owned(),
        }
    }

    /// Insert data after translation so paths, plugin identifiers and counts stay intact.
    pub fn format(self, chinese: &str, english: &str, values: &[(&str, &str)]) -> String {
        let format = self.text(chinese, english);
        let mut result = String::with_capacity(format.len());
        let mut rest = format;
        while let Some((prefix, placeholder)) = rest.split_once('{') {
            let Some((key, tail)) = placeholder.split_once('}') else {
                break;
            };
            result.push_str(prefix);
            if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
                result.push_str(value);
            } else {
                result.push('{');
                result.push_str(key);
                result.push('}');
            }
            rest = tail;
        }
        result.push_str(rest);
        result
    }
}

include!(concat!(env!("OUT_DIR"), "/ui-catalogs.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_settings_and_mcp_accept_the_same_languages() {
        assert_eq!(Language::ALL.map(Language::code), ndm_core::UI_LANGUAGES);
        for lang in Language::ALL {
            assert_eq!(Language::from_code(lang.code()), Some(lang));
            assert_eq!(
                serde_json::from_value::<Language>(serde_json::json!(lang.code())).unwrap(),
                lang
            );
            assert_eq!(serde_json::to_value(lang).unwrap(), lang.code());
            let patch = ndm_core::SettingsPatch {
                language: Some(lang.code().into()),
                ..Default::default()
            };
            assert!(patch.validate().is_ok());
        }
        assert_eq!(
            serde_json::from_str::<Language>(r#""Chinese""#).unwrap(),
            Language::Chinese
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""English""#).unwrap(),
            Language::English
        );
    }

    #[test]
    fn regional_system_tags_match_one_of_the_site_languages() {
        for (tag, expected) in [
            ("zh_HK.UTF-8", Language::TraditionalChinese),
            ("zh-Hans-TW", Language::Chinese),
            ("pt-BR", Language::BrazilianPortuguese),
            ("de-AT", Language::German),
        ] {
            assert_eq!(Language::from_system_tag(tag), Some(expected));
        }
        assert_eq!(Language::from_system_tag("sv-SE"), None);
    }

    #[test]
    fn translated_templates_do_not_interpret_inserted_data() {
        assert_eq!(
            Language::English.format("", "{b}: {a}", &[("a", "42"), ("b", "one{a}")]),
            "one{a}: 42"
        );
    }
}
