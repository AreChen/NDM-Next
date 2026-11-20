//! Metadata shared by the installer, native controls, and MCP.
use std::{collections::HashSet, io, path::Path};

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const MANIFEST_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "plugin.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized {
    pub en: String,
    #[serde(default)]
    pub zh: String,
}

impl Localized {
    pub fn for_language(&self, chinese: bool) -> &str {
        if chinese && !self.zh.is_empty() {
            &self.zh
        } else {
            &self.en
        }
    }

    fn valid(&self, limit: usize) -> bool {
        !self.en.trim().is_empty() && self.en.len() <= limit && self.zh.len() <= limit
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginManifest {
    pub schema: u32,
    pub id: String,
    pub version: Version,
    pub name: Localized,
    pub description: Localized,
    pub engine: String,
    pub engine_protocol: u32,
    pub inputs: Vec<InputMatcher>,
    pub limits: EngineLimits,
    #[serde(default)]
    pub options: Vec<OptionField>,
    #[serde(default)]
    pub panel: Option<String>,
    #[serde(default)]
    pub tools: Vec<DownloadTool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InputMatcher {
    pub schemes: Vec<String>,
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub suffixes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct EngineLimits {
    pub active_tasks: u32,
    /// Omit when the engine does not have a per-task thread setting.
    pub download_threads: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadTool {
    pub id: String,
    pub name: Localized,
    pub description: Localized,
    #[serde(default)]
    pub defaults: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OptionField {
    pub key: String,
    pub label: Localized,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub required: bool,
    #[serde(flatten)]
    pub control: OptionControl,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OptionControl {
    Boolean,
    Integer { min: i64, max: i64 },
    Text { max_length: usize },
    Choice { choices: Vec<Choice> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: Localized,
}

/// A panel script selects and labels the native controls already declared in the manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Panel {
    pub fields: Vec<PanelField>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PanelField {
    pub key: String,
    pub label: Option<Localized>,
}

impl PluginManifest {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema != MANIFEST_VERSION
            || self.engine_protocol != crate::ENGINE_PROTOCOL_VERSION
            || !valid_identifier(&self.id)
            || !self.name.valid(128)
            || !self.description.valid(1024)
            || !valid_package_path(&self.engine)
            || self
                .panel
                .as_ref()
                .is_some_and(|p| !valid_package_path(p) || !p.ends_with(".js"))
            || self.inputs.is_empty()
            || self.inputs.len() > 32
            || self.inputs.iter().any(|input| !input.valid())
            || !(1..=32).contains(&self.limits.active_tasks)
            || self
                .limits
                .download_threads
                .is_some_and(|n| !(1..=256).contains(&n))
            || self.options.len() > 24
            || self.tools.len() > 8
        {
            return Err(invalid("invalid or incompatible plugin manifest"));
        }
        let mut keys = HashSet::new();
        for field in &self.options {
            if !keys.insert(&field.key)
                || !valid_identifier(&field.key)
                || !field.label.valid(128)
                || !field.control.valid()
                || field
                    .default
                    .as_ref()
                    .is_some_and(|value| !field.control.accepts(value))
            {
                return Err(invalid("invalid plugin option definition"));
            }
        }
        let mut tools = HashSet::new();
        for tool in &self.tools {
            if !tools.insert(&tool.id)
                || !valid_identifier(&tool.id)
                || tool.id.len() > 18
                || !tool.name.valid(128)
                || !tool.description.valid(1024)
                || tool
                    .defaults
                    .iter()
                    .any(|(key, value)| !self.accepts(key, value))
            {
                return Err(invalid("invalid plugin tool definition"));
            }
        }
        Ok(())
    }

    pub fn complete_options(
        &self,
        mut values: Map<String, Value>,
    ) -> io::Result<Map<String, Value>> {
        if values.iter().any(|(key, value)| !self.accepts(key, value)) {
            return Err(invalid("invalid plugin option"));
        }
        for field in &self.options {
            if values.contains_key(&field.key) {
                continue;
            }
            if let Some(value) = &field.default {
                values.insert(field.key.clone(), value.clone());
            } else if field.required {
                return Err(invalid("required plugin option missing"));
            }
        }
        Ok(values)
    }

    pub fn option_schema(&self, tool: &DownloadTool) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();
        for field in &self.options {
            let mut schema = field.control.schema();
            schema["description"] = json!(field.label.en);
            if let Some(default) = tool.defaults.get(&field.key).or(field.default.as_ref()) {
                schema["default"] = default.clone();
            } else if field.required {
                required.push(&field.key);
            }
            properties.insert(field.key.clone(), schema);
        }
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
    }

    pub fn panel_fields(&self, panel: Panel) -> io::Result<Vec<OptionField>> {
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        for item in panel.fields {
            let mut field = self
                .options
                .iter()
                .find(|field| field.key == item.key)
                .cloned()
                .ok_or_else(|| invalid("unknown panel field"))?;
            if !seen.insert(item.key) {
                return Err(invalid("duplicate panel field"));
            }
            if let Some(label) = item.label {
                if !label.valid(128) {
                    return Err(invalid("invalid panel label"));
                }
                field.label = label;
            }
            fields.push(field);
        }
        for field in &self.options {
            if field.required && field.default.is_none() && !seen.contains(&field.key) {
                return Err(invalid("required field missing from panel"));
            }
        }
        Ok(fields)
    }

    fn accepts(&self, key: &str, value: &Value) -> bool {
        self.options
            .iter()
            .any(|field| field.key == key && field.control.accepts(value))
    }
}

impl InputMatcher {
    fn valid(&self) -> bool {
        !self.schemes.is_empty()
            && self.schemes.len() <= 16
            && self.schemes.iter().all(|s| {
                !s.is_empty()
                    && s.len() <= 32
                    && s.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b)
                    })
            })
            && self.hosts.len() <= 64
            && self
                .hosts
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 253 && s.split('.').all(valid_identifier))
            && self.suffixes.len() <= 16
            && self.suffixes.iter().all(|s| {
                s.starts_with('.')
                    && (2..=24).contains(&s.len())
                    && s.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.')
            })
    }

    pub fn score(&self, scheme: &str, host: Option<&str>, path: &str) -> Option<u8> {
        if !self.schemes.iter().any(|known| known == scheme)
            || (!self.hosts.is_empty()
                && !host.is_some_and(|host| {
                    self.hosts.iter().any(|known| {
                        host == known || host.strip_suffix(known).is_some_and(|p| p.ends_with('.'))
                    })
                }))
            || (!self.suffixes.is_empty()
                && !self.suffixes.iter().any(|suffix| path.ends_with(suffix)))
        {
            return None;
        }
        Some(1 + u8::from(!self.hosts.is_empty()) * 2 + u8::from(!self.suffixes.is_empty()))
    }
}

impl OptionControl {
    fn valid(&self) -> bool {
        match self {
            Self::Boolean => true,
            Self::Integer { min, max } => min <= max,
            Self::Text { max_length } => (1..=4096).contains(max_length),
            Self::Choice { choices } => {
                let mut values = HashSet::new();
                (1..=32).contains(&choices.len())
                    && choices.iter().all(|choice| {
                        !choice.value.is_empty()
                            && choice.value.len() <= 128
                            && choice.label.valid(128)
                            && values.insert(&choice.value)
                    })
            }
        }
    }

    pub fn accepts(&self, value: &Value) -> bool {
        match self {
            Self::Boolean => value.is_boolean(),
            Self::Integer { min, max } => {
                value.as_i64().is_some_and(|n| (*min..=*max).contains(&n))
            }
            Self::Text { max_length } => value
                .as_str()
                .is_some_and(|text| text.chars().take(max_length + 1).count() <= *max_length),
            Self::Choice { choices } => value
                .as_str()
                .is_some_and(|value| choices.iter().any(|choice| choice.value == value)),
        }
    }

    fn schema(&self) -> Value {
        match self {
            Self::Boolean => json!({"type":"boolean"}),
            Self::Integer { min, max } => json!({"type":"integer","minimum":min,"maximum":max}),
            Self::Text { max_length } => json!({"type":"string","maxLength":max_length}),
            Self::Choice { choices } => {
                json!({"type":"string","enum":choices.iter().map(|choice| &choice.value).collect::<Vec<_>>()})
            }
        }
    }
}

pub fn valid_identifier(value: &str) -> bool {
    (1..=40).contains(&value.len())
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A portable relative path with no aliasing, drive prefix, or traversal.
pub fn valid_package_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 240
        && !Path::new(path).is_absolute()
        && path.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && !matches!(
                    part.split('.')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_uppercase()
                        .as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}
