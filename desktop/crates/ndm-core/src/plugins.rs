//! Installed engine packages and the official plugin catalog.
use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use futures_util::StreamExt;
pub use ndm_plugin_sdk::{
    Choice, DownloadTool, Localized, OptionControl, OptionField, OptionValueError, Panel,
    PluginManifest, TextEditor, normalize_proxy_address, selected_text_values,
};
use ndm_plugin_sdk::{MANIFEST_FILE, valid_identifier, valid_package_path};
use reqwest::Url;
use semver::Version;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/AreChen/NDM-Next-Plugin-Market/main/catalog.json";
const JSON_LIMIT: u64 = 2 * 1024 * 1024;
const PACKAGE_LIMIT: u64 = 384 * 1024 * 1024;
const EXTRACT_LIMIT: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    #[serde(default)]
    pub categories: BTreeMap<String, Localized>,
    pub plugins: Vec<CatalogPlugin>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogPlugin {
    pub id: String,
    pub name: Localized,
    pub description: Localized,
    pub version: Version,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub artifacts: BTreeMap<String, Artifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
}

impl Catalog {
    pub async fn fetch() -> io::Result<Self> {
        let response = client()?
            .get(CATALOG_URL)
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(network)?
            .error_for_status()
            .map_err(network)?;
        let mut output = Vec::new();
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(network)?;
            if output.len().saturating_add(chunk.len()) > JSON_LIMIT as usize {
                return Err(invalid("plugin catalog is too large"));
            }
            output.extend_from_slice(&chunk);
        }
        Self::parse(&output)
    }

    fn parse(json: &[u8]) -> io::Result<Self> {
        let catalog: Self = serde_json::from_slice(json).map_err(io::Error::other)?;
        let mut ids = HashSet::new();
        if catalog.schema != 1
            || catalog.categories.len() > 16
            || catalog.categories.iter().any(|(id, name)| {
                !valid_identifier(id)
                    || name.en.trim().is_empty()
                    || name.en.len() > 96
                    || name.zh.len() > 96
            })
            || catalog.plugins.len() > 128
            || catalog.plugins.iter().any(|plugin| {
                !valid_identifier(&plugin.id)
                    || !ids.insert(&plugin.id)
                    || plugin.name.en.is_empty()
                    || plugin.name.en.len() > 128
                    || plugin.name.zh.len() > 128
                    || plugin.description.en.len() > 1024
                    || plugin.description.zh.len() > 1024
                    || plugin
                        .category
                        .as_ref()
                        .is_some_and(|id| !catalog.categories.contains_key(id))
                    || plugin.keywords.len() > 16
                    || plugin
                        .keywords
                        .iter()
                        .any(|keyword| keyword.trim().is_empty() || keyword.len() > 96)
                    || plugin.artifacts.len() > 8
                    || plugin.artifacts.values().any(|artifact| !artifact.valid())
            })
        {
            return Err(invalid("invalid plugin catalog"));
        }
        Ok(catalog)
    }
}

impl CatalogPlugin {
    pub fn artifact(&self) -> Option<&Artifact> {
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        self.artifacts.get(&platform)
    }
}

impl Artifact {
    fn valid(&self) -> bool {
        self.sha256.len() == 64
            && self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            && Url::parse(&self.url).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
    }
}

#[derive(Clone, Debug)]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    pub directory: PathBuf,
    pub option_defaults: Map<String, Value>,
}

impl InstalledPlugin {
    pub fn executable(&self) -> PathBuf {
        self.directory.join(&self.manifest.engine)
    }

    pub fn read_panel(&self) -> io::Result<Option<String>> {
        self.manifest
            .panel
            .as_ref()
            .map(|script| {
                let bytes = bounded_file(&self.directory.join(script), 64 * 1024)?;
                String::from_utf8(bytes).map_err(io::Error::other)
            })
            .transpose()
    }

    fn complete_options(&self, values: Map<String, Value>) -> io::Result<Map<String, Value>> {
        let mut defaults = self.option_defaults.clone();
        defaults.extend(values);
        self.manifest.complete_options(defaults)
    }

    pub fn option_schema(&self, tool: &DownloadTool) -> Value {
        let mut schema = self.manifest.option_schema(tool);
        if let Some(required) = schema["required"].as_array_mut() {
            required.retain(|key| {
                key.as_str()
                    .is_none_or(|key| !self.option_defaults.contains_key(key))
            });
        }
        // An agent can use the configured value by omitting the field.
        // Keep its literal value out of the tool description.
        for key in self
            .option_defaults
            .keys()
            .filter(|key| !tool.defaults.contains_key(*key))
        {
            if let Some(property) = schema["properties"][key].as_object_mut() {
                property.remove("default");
            }
        }
        schema
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InstalledPlugins {
    active: BTreeMap<String, String>,
    #[serde(default)]
    preferred: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    option_defaults: BTreeMap<String, StoredDefaults>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum StoredDefaults {
    Protected(Vec<u8>),
    Plain(Map<String, Value>),
}

impl StoredDefaults {
    fn read(&self) -> io::Result<Map<String, Value>> {
        match self {
            Self::Protected(bytes) => {
                let decoded = crate::secrets::unprotect(bytes).map_err(io::Error::other)?;
                serde_json::from_slice(&decoded).map_err(io::Error::other)
            }
            Self::Plain(values) => Ok(values.clone()),
        }
    }

    fn protect(&mut self) -> io::Result<()> {
        if let Self::Plain(values) = self {
            let bytes = serde_json::to_vec(values).map_err(io::Error::other)?;
            *self = Self::Protected(crate::secrets::protect(&bytes).map_err(io::Error::other)?);
        }
        Ok(())
    }
}

impl InstalledPlugins {
    fn set_defaults(&mut self, id: &str, defaults: Map<String, Value>) {
        if defaults.is_empty() {
            self.option_defaults.remove(id);
        } else {
            self.option_defaults
                .insert(id.to_owned(), StoredDefaults::Plain(defaults));
        }
    }
}

#[derive(Clone)]
pub struct PluginRegistry {
    root: PathBuf,
}

impl PluginRegistry {
    pub fn new(profile: &Path) -> Self {
        Self {
            root: profile.join("plugins"),
        }
    }

    fn load(&self) -> io::Result<InstalledPlugins> {
        match read_json(&self.root.join("installed.json")) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(InstalledPlugins::default())
            }
            Err(error) => Err(error),
        }
    }

    fn save(&self, snapshot: &mut InstalledPlugins) -> io::Result<()> {
        for defaults in snapshot.option_defaults.values_mut() {
            defaults.protect()?;
        }
        fs::create_dir_all(&self.root)?;
        let mut pending = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(&mut pending, snapshot).map_err(io::Error::other)?;
        if pending.as_file().metadata()?.len() > JSON_LIMIT {
            return Err(invalid("plugin registry exceeds its size limit"));
        }
        pending.as_file().sync_all()?;
        pending
            .persist(self.root.join("installed.json"))
            .map_err(io::Error::other)?;
        Ok(())
    }

    fn mutation_lock(&self) -> io::Result<fs::File> {
        fs::create_dir_all(&self.root)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("registry.lock"))?;
        file.lock()?;
        Ok(file)
    }

    fn installed(
        &self,
        id: &str,
        revision: &str,
        snapshot: &InstalledPlugins,
    ) -> io::Result<InstalledPlugin> {
        if !valid_identifier(id) || !valid_package_path(revision) || revision.contains('/') {
            return Err(invalid("invalid installed plugin reference"));
        }
        let directory = self.root.join(id).join(revision);
        crate::cache::validate_directory(&directory)?;
        let manifest: PluginManifest = read_json(&directory.join(MANIFEST_FILE))?;
        manifest.validate()?;
        if manifest.id != id {
            return Err(invalid("invalid plugin package identity"));
        }
        let option_defaults = validated_defaults(&manifest, snapshot).unwrap_or_default();
        Ok(InstalledPlugin {
            manifest,
            directory,
            option_defaults,
        })
    }

    pub fn list(&self) -> io::Result<Vec<InstalledPlugin>> {
        let snapshot = self.load()?;
        snapshot
            .active
            .iter()
            .map(|(id, revision)| self.installed(id, revision, &snapshot))
            .collect()
    }

    pub fn get(&self, id: &str) -> io::Result<Option<InstalledPlugin>> {
        let snapshot = self.load()?;
        snapshot
            .active
            .get(id)
            .map(|revision| self.installed(id, revision, &snapshot))
            .transpose()
    }

    /// The most specific matcher wins. A saved site choice has precedence.
    pub fn candidates(&self, source: &Url) -> io::Result<Vec<InstalledPlugin>> {
        let snapshot = self.load()?;
        let host = source.host_str().map(|value| value.trim_end_matches('.'));
        let path = source.path().to_ascii_lowercase();
        let preferred = host.and_then(|host| snapshot.preferred.get(host));
        let mut matches = Vec::new();
        for (id, revision) in &snapshot.active {
            let Ok(plugin) = self.installed(id, revision, &snapshot) else {
                continue;
            };
            if let Some(score) = plugin
                .manifest
                .inputs
                .iter()
                .filter_map(|input| input.score(source.scheme(), host, &path))
                .max()
            {
                matches.push((preferred == Some(id), score, plugin));
            }
        }
        matches.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then(a.2.manifest.id.cmp(&b.2.manifest.id))
        });
        Ok(matches.into_iter().map(|(_, _, plugin)| plugin).collect())
    }

    pub fn remember(&self, source: &Url, id: &str) -> io::Result<()> {
        let Some(host) = source.host_str() else {
            return Ok(());
        };
        let _lock = self.mutation_lock()?;
        let mut snapshot = self.load()?;
        if !snapshot.active.contains_key(id) {
            return Err(invalid("plugin is no longer installed"));
        }
        snapshot
            .preferred
            .insert(host.trim_end_matches('.').to_owned(), id.to_owned());
        self.save(&mut snapshot)
    }

    /// Save defaults for this installed revision. Each task can override them.
    pub fn save_option_defaults(
        &self,
        plugin: &InstalledPlugin,
        defaults: Map<String, Value>,
    ) -> io::Result<InstalledPlugin> {
        let _lock = self.mutation_lock()?;
        let mut snapshot = self.load()?;
        let id = &plugin.manifest.id;
        let revision = snapshot
            .active
            .get(id)
            .ok_or_else(|| invalid("plugin is no longer installed"))?;
        let mut installed = self.installed(id, revision, &snapshot)?;
        if installed.directory != plugin.directory {
            return Err(invalid("installed plugin revision changed"));
        }
        let mut defaults = installed
            .manifest
            .normalize_defaults(defaults)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        defaults.retain(|key, value| {
            !installed
                .manifest
                .options
                .iter()
                .any(|field| field.key == *key && field.default.as_ref() == Some(value))
        });
        snapshot.set_defaults(id, defaults.clone());
        self.save(&mut snapshot)?;
        installed.option_defaults = defaults;
        Ok(installed)
    }

    pub fn task_options(
        &self,
        source: &Url,
        engine: Option<&str>,
        values: Map<String, Value>,
    ) -> io::Result<Option<crate::TaskOptions>> {
        let candidates = self.candidates(source)?;
        let selected = match engine {
            Some(engine) => Some(
                candidates
                    .into_iter()
                    .find(|plugin| plugin.manifest.id == engine)
                    .ok_or_else(|| invalid("plugin cannot handle source"))?,
            ),
            None => candidates.into_iter().next(),
        };
        match selected {
            Some(plugin) => {
                let values = plugin.complete_options(values)?;
                Ok(Some(crate::TaskOptions::Plugin(crate::PluginOptions {
                    id: plugin.manifest.id,
                    values,
                })))
            }
            None if values.is_empty() => Ok(None),
            None => Err(invalid("no plugin handles these options")),
        }
    }

    pub async fn download_and_install(
        &self,
        plugin: &CatalogPlugin,
    ) -> io::Result<InstalledPlugin> {
        let artifact = plugin
            .artifact()
            .filter(|artifact| artifact.valid())
            .ok_or_else(|| invalid("package unavailable for this platform"))?;
        let response = client()?
            .get(&artifact.url)
            .send()
            .await
            .map_err(network)?
            .error_for_status()
            .map_err(network)?;
        if response
            .content_length()
            .is_some_and(|length| length > PACKAGE_LIMIT)
        {
            return Err(invalid("plugin package is too large"));
        }
        tokio::fs::create_dir_all(&self.root).await?;
        let pending = tempfile::NamedTempFile::new_in(&self.root)?;
        let mut file = tokio::fs::File::from_std(pending.reopen()?);
        let mut body = response.bytes_stream();
        let mut length = 0u64;
        let mut digest = Sha256::new();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(network)?;
            length += chunk.len() as u64;
            if length > PACKAGE_LIMIT {
                return Err(invalid("plugin package is too large"));
            }
            use tokio::io::AsyncWriteExt;
            file.write_all(&chunk).await?;
            digest.update(&chunk);
        }
        file.sync_all().await?;
        drop(file);
        if !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(&artifact.sha256) {
            return Err(invalid("plugin package checksum mismatch"));
        }
        let registry = self.clone();
        let id = plugin.id.clone();
        let version = plugin.version.clone();
        tokio::task::spawn_blocking(move || registry.install(pending.path(), Some((&id, &version))))
            .await
            .map_err(io::Error::other)?
    }

    pub fn import(&self, package: &Path) -> io::Result<InstalledPlugin> {
        self.install(package, None)
    }

    fn install(
        &self,
        package: &Path,
        expected: Option<(&str, &Version)>,
    ) -> io::Result<InstalledPlugin> {
        let file = fs::File::open(package)?;
        if file.metadata()?.len() > PACKAGE_LIMIT {
            return Err(invalid("plugin package is too large"));
        }
        fs::create_dir_all(&self.root)?;
        let staging = tempfile::tempdir_in(&self.root)?;
        unpack(file, staging.path())?;
        let manifest: PluginManifest = read_json(&staging.path().join(MANIFEST_FILE))?;
        manifest.validate()?;
        if expected.is_some_and(|(id, version)| manifest.id != id || &manifest.version != version) {
            return Err(invalid("plugin package does not match catalog"));
        }
        let engine = staging.path().join(&manifest.engine);
        if !fs::metadata(&engine)?.is_file()
            || (cfg!(windows) && !manifest.engine.ends_with(".exe"))
        {
            return Err(invalid("missing plugin executable"));
        }
        if let Some(panel) = &manifest.panel {
            bounded_file(&staging.path().join(panel), 64 * 1024)?;
        }
        make_executable(&engine)?;
        let _lock = self.mutation_lock()?;
        let mut snapshot = self.load()?;
        let revision = crate::TaskId::new_v4().to_string();
        let versions = self.root.join(&manifest.id);
        fs::create_dir_all(&versions)?;
        let directory = versions.join(&revision);
        fs::rename(staging.path(), &directory)?;
        snapshot.active.insert(manifest.id.clone(), revision);
        let option_defaults = validated_defaults(&manifest, &snapshot);
        if let Some(defaults) = &option_defaults {
            snapshot.set_defaults(&manifest.id, defaults.clone());
        }
        if let Err(error) = self.save(&mut snapshot) {
            let _ = fs::remove_dir_all(&directory);
            return Err(error);
        }
        Ok(InstalledPlugin {
            manifest,
            directory,
            option_defaults: option_defaults.unwrap_or_default(),
        })
    }

    /// Call after download dispatch has established the engine is inactive.
    pub(crate) fn remove(&self, id: &str) -> io::Result<()> {
        let _lock = self.mutation_lock()?;
        let mut snapshot = self.load()?;
        if !valid_identifier(id) || snapshot.active.remove(id).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "plugin is not installed",
            ));
        }
        snapshot.preferred.retain(|_, preferred| preferred != id);
        snapshot.option_defaults.remove(id);
        self.save(&mut snapshot)?;
        fs::remove_dir_all(self.root.join(id))
    }
}

fn validated_defaults(
    manifest: &PluginManifest,
    snapshot: &InstalledPlugins,
) -> Option<Map<String, Value>> {
    let mut defaults = snapshot.option_defaults.get(&manifest.id)?.read().ok()?;
    defaults.retain(|key, value| manifest.accepts_option(key, value));
    Some(defaults)
}

pub fn parse_source(input: &str) -> io::Result<Url> {
    let input = input.trim();
    if input.is_empty() || input.len() > 64 * 1024 || input.chars().any(char::is_control) {
        return Err(invalid("invalid download source"));
    }
    let path = Path::new(input);
    if path.is_absolute() {
        Url::from_file_path(path).map_err(|()| invalid("invalid file path"))
    } else {
        Url::parse(input).map_err(|_| invalid("invalid download address"))
    }
}

pub fn built_in_source(source: &Url) -> bool {
    matches!(source.scheme(), "http" | "https" | "ftp" | "ftps" | "sftp")
        && source.host_str().is_some()
}

fn client() -> io::Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(300))
        .user_agent("NDM-Next")
        .build()
        .map_err(network)
}

fn network(_: reqwest::Error) -> io::Error {
    io::Error::other("could not fetch plugin catalog or package")
}

fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&bounded_file(path, JSON_LIMIT)?).map_err(io::Error::other)
}

fn bounded_file(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || crate::cache::is_reparse(&metadata) || metadata.len() > limit {
        return Err(invalid("invalid plugin file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("plugin file exceeds its limit"));
    }
    Ok(bytes)
}

fn unpack(file: fs::File, directory: &Path) -> io::Result<()> {
    let mut archive = zip::ZipArchive::new(file).map_err(io::Error::other)?;
    if archive.is_empty() || archive.len() > 2048 {
        return Err(invalid("invalid plugin archive file count"));
    }
    let mut size = 0u64;
    let mut paths = HashSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(io::Error::other)?;
        let name = entry.name().map_err(io::Error::other)?;
        let relative = name.trim_end_matches('/');
        if !valid_package_path(relative)
            || !paths.insert(relative.to_ascii_lowercase())
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err(invalid("unsafe file in plugin archive"));
        }
        let path = directory.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(path)?;
            continue;
        }
        size = size
            .checked_add(entry.size())
            .filter(|size| *size <= EXTRACT_LIMIT)
            .ok_or_else(|| invalid("expanded plugin package is too large"))?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("invalid plugin path"))?;
        fs::create_dir_all(parent)?;
        let mut output = fs::File::create_new(&path)?;
        let expected = entry.size();
        if io::copy(
            &mut Read::by_ref(&mut entry).take(expected + 1),
            &mut output,
        )? != expected
        {
            return Err(invalid("invalid expanded plugin file size"));
        }
        output.flush()?;
        if entry.unix_mode().is_some_and(|mode| mode & 0o111 != 0) {
            make_executable(&path)?;
        }
    }
    Ok(())
}

fn make_executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

#[cfg(test)]
mod tests {
    use super::{Catalog, Map, Path, PluginRegistry, Url, Value};
    use crate::TaskOptions;
    use serde_json::json;
    use std::{fs::File, io::Write};

    #[test]
    fn catalog_accepts_old_listings_and_validates_category_references() {
        let mut input = json!({
            "schema": 1,
            "plugins": [{
                "id": "yt-dlp",
                "name": {"en": "YouTube"},
                "description": {"en": "Download video"},
                "version": "1.0.0",
                "artifacts": {}
            }]
        });
        let read = |input: &serde_json::Value| Catalog::parse(input.to_string().as_bytes());
        let old = read(&input).unwrap();
        assert!(old.categories.is_empty());
        assert_eq!(old.plugins[0].category, None);
        assert!(old.plugins[0].keywords.is_empty());
        input["plugins"][0]["category"] = json!("video-audio");
        assert!(read(&input).is_err());
        input["categories"] = json!({"video-audio":{"en":"Video & audio","zh":"视频与音频"}});
        let new = read(&input).unwrap();
        assert_eq!(
            new.categories["video-audio"].for_language(true),
            "视频与音频"
        );
    }

    fn write_package(path: &Path, manifest: &Value) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("plugin.json", options).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.start_file(manifest["engine"].as_str().unwrap(), options)
            .unwrap();
        zip.write_all(b"fixture").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn plugin_defaults_persist_and_follow_the_installed_schema() {
        let profile = tempfile::tempdir().unwrap();
        let registry = PluginRegistry::new(profile.path());
        let archive = profile.path().join("plugin.zip");
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../ndm-plugin-sdk/examples/file-copy.json"))
                .unwrap();
        if cfg!(windows) {
            manifest["engine"] = json!("file-copy.exe");
        }
        manifest["inputs"] = json!([{"schemes": ["https"], "hosts": ["example.test"]}]);
        manifest["options"] = json!([
            {
                "key": "pieces", "kind": "integer", "min": 1, "max": 16,
                "label": {"en": "Pieces"}, "default": 4
            },
            {"key": "token", "kind": "text", "max_length": 128, "label": {"en": "Token"}}
        ]);
        write_package(&archive, &manifest);
        let original = registry.import(&archive).unwrap();
        let source = Url::parse("https://example.test/example").unwrap();
        let defaults = |pieces| Map::from_iter([("pieces".into(), json!(pieces))]);
        let resolved = |values| {
            let TaskOptions::Plugin(options) = PluginRegistry::new(profile.path())
                .task_options(&source, None, values)
                .unwrap()
                .unwrap()
            else {
                panic!("expected plugin options");
            };
            options.values["pieces"].as_i64().unwrap()
        };
        assert_eq!(resolved(Map::new()), 4);
        let mut values = defaults(8);
        values.insert("token".into(), json!("private-test-value"));
        let saved = registry.save_option_defaults(&original, values).unwrap();
        assert_eq!(resolved(Map::new()), 8);
        assert_eq!(resolved(defaults(2)), 2);
        assert_eq!(
            registry.get("file-copy").unwrap().unwrap().option_defaults["token"],
            "private-test-value",
        );
        let persisted =
            std::fs::read_to_string(profile.path().join("plugins/installed.json")).unwrap();
        assert!(!persisted.contains("private-test-value"));
        let persisted: Value = serde_json::from_str(&persisted).unwrap();
        assert!(persisted["option_defaults"]["file-copy"].is_array());
        let mut tool = saved.manifest.tools[0].clone();
        let schema = saved.option_schema(&tool);
        assert!(schema["properties"]["pieces"].get("default").is_none());
        assert!(!schema.to_string().contains("private-test-value"));
        tool.defaults = defaults(2);
        assert_eq!(
            saved.option_schema(&tool)["properties"]["pieces"]["default"],
            2
        );
        assert!(
            registry
                .save_option_defaults(&original, defaults(17))
                .is_err()
        );
        assert_eq!(resolved(Map::new()), 8);

        registry
            .save_option_defaults(&original, Map::new())
            .unwrap();
        assert_eq!(resolved(Map::new()), 4);
        registry
            .save_option_defaults(&original, defaults(8))
            .unwrap();
        manifest["version"] = json!("0.2.0");
        manifest["options"][0]["max"] = json!(6);
        write_package(&archive, &manifest);
        let updated = registry.import(&archive).unwrap();
        assert!(updated.option_defaults.is_empty());
        assert_eq!(resolved(Map::new()), 4);
        assert!(
            registry
                .save_option_defaults(&original, defaults(2))
                .is_err()
        );

        registry
            .save_option_defaults(&updated, defaults(3))
            .unwrap();
        registry.remove("file-copy").unwrap();
        registry.import(&archive).unwrap();
        assert_eq!(resolved(Map::new()), 4);
    }
}
