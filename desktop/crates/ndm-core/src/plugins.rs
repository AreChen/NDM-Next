//! Installed engine packages and the official plugin catalog.
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use futures_util::StreamExt;
pub use ndm_plugin_sdk::{
    Choice, DownloadTool, Localized, OptionControl, OptionField, OptionValueError, Panel,
    PluginManifest, SourceFiles, SourceSelection, TextEditor, normalize_proxy_address,
    selected_text_values,
};
use ndm_plugin_sdk::{InputMatcher, MANIFEST_FILE, valid_identifier, valid_package_path};
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
const SKILL_LIMIT: u64 = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageProgress {
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Installing { extracted: u64, total: Option<u64> },
}

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
    pub ai_enabled: bool,
}

impl InstalledPlugin {
    pub fn matches_browser_page(&self, source: &Url) -> bool {
        if !self.manifest.capabilities.browser_page || !matches!(source.scheme(), "http" | "https")
        {
            return false;
        }
        let host = source.host_str().map(|host| host.trim_end_matches('.'));
        let path = source.path().to_ascii_lowercase();
        self.manifest
            .inputs
            .iter()
            .chain(&self.manifest.manual_inputs)
            .any(|input| input.score(source.scheme(), host, &path).is_some())
    }

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

    pub fn read_skill(&self) -> io::Result<Option<String>> {
        self.manifest
            .skill
            .as_deref()
            .map(|path| read_skill(&self.directory, path))
            .transpose()
    }

    pub async fn inspect_source(
        &self,
        source: Url,
        options: Map<String, Value>,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> crate::Result<Option<SourceFiles>> {
        if !self.manifest.capabilities.inspect_files {
            return Err(crate::Error::PluginOptions);
        }
        let temporary = tempfile::tempdir()?;
        let request = ndm_plugin_sdk::DownloadRequest {
            task_id: crate::TaskId::new_v4(),
            source: crate::plugin_transfer::download_source(source, BTreeMap::new())?,
            workspace: std::path::absolute(temporary.path())?,
            suggested_filename: None,
            expected_size: None,
            download_threads: None,
            options,
            selection: None,
        };
        crate::plugin_engine::PluginEngine::new(self.executable())?
            .inspect(request, cancel)
            .await
            .map_err(|_| crate::Error::PluginSource)
    }

    fn complete_options(&self, values: Map<String, Value>) -> io::Result<Map<String, Value>> {
        let mut defaults = self.option_defaults.clone();
        defaults.extend(values);
        self.manifest.complete_options(defaults)
    }

    pub fn option_schema(&self, tool: &DownloadTool) -> Value {
        self.apply_saved_options(self.manifest.option_schema(tool), &tool.defaults)
    }

    pub fn default_option_schema(&self) -> Value {
        self.apply_saved_options(self.manifest.default_option_schema(), &Map::new())
    }

    fn apply_saved_options(&self, mut schema: Value, overrides: &Map<String, Value>) -> Value {
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
            .filter(|key| !overrides.contains_key(*key))
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
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    ai_disabled: BTreeSet<String>,
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
            ai_enabled: !snapshot.ai_disabled.contains(id),
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
        Ok(self
            .matching_plugins(source, false)?
            .into_iter()
            .map(|(_, plugin)| plugin)
            .collect())
    }

    /// Offer explicitly selectable engines along with the automatic choice.
    pub fn draft_candidates(
        &self,
        source: &Url,
    ) -> io::Result<(Vec<InstalledPlugin>, Option<String>)> {
        let candidates = self.matching_plugins(source, true)?;
        let selected = candidates
            .first()
            .filter(|(automatic, _)| *automatic)
            .map(|(_, plugin)| plugin.manifest.id.clone());
        Ok((
            candidates.into_iter().map(|(_, plugin)| plugin).collect(),
            selected,
        ))
    }

    /// Candidates for an AI request, in the same order as desktop engine selection.
    /// The boolean marks an automatic match.
    pub fn ai_candidates(&self, source: &Url) -> io::Result<Vec<(bool, InstalledPlugin)>> {
        Ok(self
            .matching_plugins(source, true)?
            .into_iter()
            .filter(|(_, plugin)| plugin.ai_enabled)
            .collect())
    }

    pub fn set_ai_access(&self, id: &str, enabled: bool) -> io::Result<()> {
        let _lock = self.mutation_lock()?;
        let mut snapshot = self.load()?;
        if !snapshot.active.contains_key(id) {
            return Err(invalid("plugin is no longer installed"));
        }
        let changed = if enabled {
            snapshot.ai_disabled.remove(id)
        } else {
            snapshot.ai_disabled.insert(id.to_owned())
        };
        if changed {
            self.save(&mut snapshot)?;
        }
        Ok(())
    }

    fn matching_plugins(
        &self,
        source: &Url,
        include_manual: bool,
    ) -> io::Result<Vec<(bool, InstalledPlugin)>> {
        let snapshot = self.load()?;
        let host = source.host_str().map(|value| value.trim_end_matches('.'));
        let path = source.path().to_ascii_lowercase();
        let scheme = source.scheme();
        let preferred = host.and_then(|host| snapshot.preferred.get(host));
        let mut matches = Vec::new();
        for (id, revision) in &snapshot.active {
            let Ok(plugin) = self.installed(id, revision, &snapshot) else {
                continue;
            };
            let automatic = input_score(&plugin.manifest.inputs, scheme, host, &path);
            let score = automatic.or_else(|| {
                include_manual
                    .then(|| input_score(&plugin.manifest.manual_inputs, scheme, host, &path))
                    .flatten()
            });
            if let Some(score) = score {
                matches.push((automatic.is_some(), preferred == Some(id), score, plugin));
            }
        }
        matches.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then(b.2.cmp(&a.2))
                .then(a.3.manifest.id.cmp(&b.3.manifest.id))
        });
        Ok(matches
            .into_iter()
            .map(|(automatic, _, _, plugin)| (automatic, plugin))
            .collect())
    }

    pub fn remember(&self, source: &Url, id: &str) -> io::Result<()> {
        let Some(host) = source.host_str() else {
            return Ok(());
        };
        let _lock = self.mutation_lock()?;
        let mut snapshot = self.load()?;
        let revision = snapshot
            .active
            .get(id)
            .ok_or_else(|| invalid("plugin is no longer installed"))?;
        let plugin = self.installed(id, revision, &snapshot)?;
        let host = host.trim_end_matches('.');
        if input_score(
            &plugin.manifest.inputs,
            source.scheme(),
            Some(host),
            &source.path().to_ascii_lowercase(),
        )
        .is_none()
        {
            return Ok(());
        }
        snapshot.preferred.insert(host.to_owned(), id.to_owned());
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
        self.resolve_task_options(source, engine, values, false)
    }

    pub fn ai_task_options(
        &self,
        source: &Url,
        engine: Option<&str>,
        values: Map<String, Value>,
    ) -> io::Result<Option<crate::TaskOptions>> {
        self.resolve_task_options(source, engine, values, true)
    }

    fn resolve_task_options(
        &self,
        source: &Url,
        engine: Option<&str>,
        values: Map<String, Value>,
        ai_only: bool,
    ) -> io::Result<Option<crate::TaskOptions>> {
        let mut candidates = self
            .matching_plugins(source, engine.is_some())?
            .into_iter()
            .map(|(_, plugin)| plugin)
            .filter(|plugin| !ai_only || plugin.ai_enabled);
        let selected = match engine {
            Some(engine) => Some(
                candidates
                    .find(|plugin| plugin.manifest.id == engine)
                    .ok_or_else(|| invalid("plugin cannot handle source"))?,
            ),
            None => candidates.next(),
        };
        match selected {
            Some(plugin) => {
                let values = plugin.complete_options(values)?;
                Ok(Some(crate::TaskOptions::Plugin(crate::PluginOptions {
                    id: plugin.manifest.id,
                    values,
                    selection: None,
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
        self.download_and_install_with_progress(plugin, |_| {})
            .await
    }

    pub async fn download_and_install_with_progress(
        &self,
        plugin: &CatalogPlugin,
        mut report: impl FnMut(PackageProgress) + Send + 'static,
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
        let total = response.content_length();
        if total.is_some_and(|length| length > PACKAGE_LIMIT) {
            return Err(invalid("plugin package is too large"));
        }
        report(PackageProgress::Downloading { received: 0, total });
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
            report(PackageProgress::Downloading {
                received: length,
                total,
            });
        }
        report(PackageProgress::Verifying);
        file.sync_all().await?;
        drop(file);
        if !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(&artifact.sha256) {
            return Err(invalid("plugin package checksum mismatch"));
        }
        let registry = self.clone();
        let id = plugin.id.clone();
        let version = plugin.version.clone();
        tokio::task::spawn_blocking(move || {
            registry.install(pending.path(), Some((&id, &version)), report)
        })
        .await
        .map_err(io::Error::other)?
    }

    pub fn import(&self, package: &Path) -> io::Result<InstalledPlugin> {
        self.import_with_progress(package, |_| {})
    }

    pub fn import_with_progress(
        &self,
        package: &Path,
        report: impl FnMut(PackageProgress),
    ) -> io::Result<InstalledPlugin> {
        self.install(package, None, report)
    }

    fn install(
        &self,
        package: &Path,
        expected: Option<(&str, &Version)>,
        report: impl FnMut(PackageProgress),
    ) -> io::Result<InstalledPlugin> {
        let file = fs::File::open(package)?;
        if file.metadata()?.len() > PACKAGE_LIMIT {
            return Err(invalid("plugin package is too large"));
        }
        fs::create_dir_all(&self.root)?;
        let staging = tempfile::tempdir_in(&self.root)?;
        unpack(file, staging.path(), report)?;
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
        if let Some(skill) = &manifest.skill {
            read_skill(staging.path(), skill)?;
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
            ai_enabled: !snapshot.ai_disabled.contains(&manifest.id),
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
        snapshot.ai_disabled.remove(id);
        self.save(&mut snapshot)?;
        fs::remove_dir_all(self.root.join(id))
    }
}

fn input_score(
    inputs: &[InputMatcher],
    scheme: &str,
    host: Option<&str>,
    path: &str,
) -> Option<u8> {
    inputs
        .iter()
        .filter_map(|input| input.score(scheme, host, path))
        .max()
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

fn read_skill(directory: &Path, relative: &str) -> io::Result<String> {
    if !valid_package_path(relative) || !relative.ends_with(".md") {
        return Err(invalid("invalid plugin Skill path"));
    }
    let mut path = directory.to_owned();
    for part in relative.split('/') {
        crate::cache::validate_directory(&path)?;
        path.push(part);
    }
    let text = String::from_utf8(bounded_file(&path, SKILL_LIMIT)?).map_err(io::Error::other)?;
    if text.trim().is_empty() {
        return Err(invalid("empty plugin Skill"));
    }
    Ok(text)
}

fn unpack(
    file: fs::File,
    directory: &Path,
    mut report: impl FnMut(PackageProgress),
) -> io::Result<()> {
    let mut archive = zip::ZipArchive::new(file).map_err(io::Error::other)?;
    if archive.is_empty() || archive.len() > 2048 {
        return Err(invalid("invalid plugin archive file count"));
    }
    let total = archive
        .decompressed_size()
        .map(|bytes| {
            if bytes > u128::from(EXTRACT_LIMIT) {
                Err(invalid("expanded plugin package is too large"))
            } else {
                Ok(bytes as u64)
            }
        })
        .transpose()?;
    let mut extracted = 0u64;
    report(PackageProgress::Installing { extracted, total });
    let mut size = 0u64;
    let mut buffer = [0; 64 * 1024];
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
        let mut limited = Read::by_ref(&mut entry).take(expected + 1);
        let mut written = 0;
        loop {
            let count = limited.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            written += count as u64;
            if written > expected {
                return Err(invalid("invalid expanded plugin file size"));
            }
            output.write_all(&buffer[..count])?;
            extracted += count as u64;
            report(PackageProgress::Installing { extracted, total });
        }
        if written != expected {
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
    use super::{Catalog, Map, PackageProgress, Path, PluginRegistry, Url, Value};
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
    fn manual_engine_routes_leave_automatic_file_downloads_available() {
        let profile = tempfile::tempdir().unwrap();
        let registry = PluginRegistry::new(profile.path());
        let archive = profile.path().join("plugin.zip");
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../ndm-plugin-sdk/examples/file-copy.json"))
                .unwrap();
        if cfg!(windows) {
            manifest["engine"] = json!("file-copy.exe");
        }
        manifest["inputs"] = json!([{
            "schemes": ["https"], "hosts": ["media.example.test"],
            "exclude_suffixes": [".mp4", ".m3u8", ".mpd"]
        }]);
        manifest["manual_inputs"] = json!([{"schemes": ["https"]}]);
        write_package(&archive, &manifest);
        let id = registry.import(&archive).unwrap().manifest.id;

        let site = Url::parse("https://video.media.example.test./watch").unwrap();
        let (available, selected) = registry.draft_candidates(&site).unwrap();
        assert_eq!(available.len(), 1);
        assert_eq!(selected.as_ref(), Some(&id));
        assert!(matches!(
            registry.task_options(&site, None, Map::new()).unwrap(),
            Some(TaskOptions::Plugin(plugin)) if plugin.id == id
        ));

        for url in [
            "https://another.example.test/watch",
            "https://media.example.test/clip.MP4",
            "https://media.example.test/live.m3u8",
            "https://media.example.test/video.mpd?token=fixture",
        ] {
            let source = Url::parse(url).unwrap();
            let (available, selected) = registry.draft_candidates(&source).unwrap();
            assert_eq!(available.len(), 1);
            assert_eq!(available[0].manifest.id, id);
            assert!(selected.is_none());
            assert!(matches!(
                registry.task_options(&source, Some(&id), Map::new()).unwrap(),
                Some(TaskOptions::Plugin(plugin)) if plugin.id == id
            ));
            registry.remember(&source, &id).unwrap();
            assert!(registry.candidates(&source).unwrap().is_empty());
            assert!(
                registry
                    .task_options(&source, None, Map::new())
                    .unwrap()
                    .is_none()
            );
        }

        let ftp = Url::parse("ftp://media.example.test/clip").unwrap();
        assert!(registry.task_options(&ftp, Some(&id), Map::new()).is_err());
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
        let mut reports = Vec::new();
        let original = registry
            .import_with_progress(&archive, |progress| reports.push(progress))
            .unwrap();
        assert!(matches!(
            reports.first(),
            Some(PackageProgress::Installing { extracted: 0, .. })
        ));
        assert!(
            matches!(reports.last(), Some(PackageProgress::Installing { extracted, total: Some(total) }) if extracted == total && *total > 0)
        );
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
