use super::*;
use ndm_plugin_sdk::RuntimeDependency;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeDependencyError {
    #[error("{consumer} needs {provider} {version} with the {tool} executable")]
    Missing {
        consumer: String,
        provider: String,
        version: semver::VersionReq,
        tool: String,
    },
    #[error("{provider} is required by {consumer}")]
    RequiredBy { provider: String, consumer: String },
    #[error("an active worker is using tools from {0}")]
    Busy(String),
    #[error("plugin runtime dependencies form a cycle")]
    Cycle,
}

impl RuntimeDependencyError {
    pub fn localized(&self, chinese: bool) -> String {
        if !chinese {
            return self.to_string();
        }
        match self {
            Self::Missing {
                consumer,
                provider,
                version,
                tool,
            } => format!(
                "插件 {consumer} 需要 {provider} {version} 提供的 {tool}，请检查提供方的安装和版本。"
            ),
            Self::RequiredBy { provider, consumer } => {
                format!("插件 {consumer} 正在依赖 {provider}，请先移除 {consumer}。")
            }
            Self::Busy(provider) => format!("正在运行的任务仍在使用 {provider} 的工具。"),
            Self::Cycle => "插件运行工具存在循环依赖。".into(),
        }
    }
}

#[derive(Default)]
pub(crate) struct ResolvedRuntimes {
    pub(crate) paths: BTreeMap<String, PathBuf>,
    // Workers retain their provider revisions, including during compatible updates.
    leases: Vec<fs::File>,
}

fn missing(consumer: &str, dep: &RuntimeDependency) -> io::Error {
    io::Error::other(RuntimeDependencyError::Missing {
        consumer: consumer.into(),
        provider: dep.plugin.clone(),
        version: dep.version.clone(),
        tool: dep.name.clone(),
    })
}

fn runtime_file(
    consumer: &str,
    dep: &RuntimeDependency,
    version: &Version,
    directory: &Path,
) -> io::Result<PathBuf> {
    if !dep.version.matches(version) {
        return Err(missing(consumer, dep));
    }
    let path = directory.join(&dep.path);
    if let Some(parents) = Path::new(&dep.path).parent() {
        let mut dir = directory.to_owned();
        for part in parents.components() {
            dir.push(part);
            crate::cache::validate_directory(&dir).map_err(|_| missing(consumer, dep))?;
        }
    }
    let metadata = fs::symlink_metadata(&path).map_err(|_| missing(consumer, dep))?;
    if !metadata.is_file() || crate::cache::is_reparse(&metadata) {
        return Err(missing(consumer, dep));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(missing(consumer, dep));
        }
    }
    #[cfg(windows)]
    if !dep.path.to_ascii_lowercase().ends_with(".exe") {
        return Err(missing(consumer, dep));
    }
    Ok(path)
}

impl PluginRegistry {
    fn runtime_lease(&self, provider: &str) -> io::Result<fs::File> {
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join(provider).join("runtime-users.lock"))
    }

    pub(super) fn resolve_runtimes(
        &self,
        consumer: &InstalledPlugin,
    ) -> io::Result<ResolvedRuntimes> {
        let _lock = self.mutation_lock()?;
        let snapshot = self.load()?;
        let mut runtimes = ResolvedRuntimes::default();
        let mut leased = BTreeSet::new();
        for dep in &consumer.manifest.runtime_dependencies {
            let id = &consumer.manifest.id;
            let revision = snapshot
                .active
                .get(&dep.plugin)
                .ok_or_else(|| missing(id, dep))?;
            let provider = self.installed(&dep.plugin, revision, &snapshot)?;
            let path = runtime_file(id, dep, &provider.manifest.version, &provider.directory)?;
            if leased.insert(&dep.plugin) {
                let lease = self.runtime_lease(&dep.plugin)?;
                lease.lock_shared()?;
                runtimes.leases.push(lease);
            }
            runtimes.paths.insert(dep.name.clone(), path);
        }
        Ok(runtimes)
    }

    pub(super) fn check_runtime_install(
        &self,
        snapshot: &InstalledPlugins,
        candidate: &PluginManifest,
        staging: &Path,
    ) -> io::Result<()> {
        let mut packages = BTreeMap::new();
        for (id, revision) in &snapshot.active {
            if id != &candidate.id {
                let plugin = self.installed(id, revision, snapshot)?;
                packages.insert(id.clone(), (plugin.manifest, plugin.directory));
            }
        }
        packages.insert(candidate.id.clone(), (candidate.clone(), staging.into()));
        for (id, (manifest, _)) in &packages {
            for dep in &manifest.runtime_dependencies {
                if id != &candidate.id && dep.plugin != candidate.id {
                    continue;
                }
                let (provider, directory) =
                    packages.get(&dep.plugin).ok_or_else(|| missing(id, dep))?;
                runtime_file(id, dep, &provider.version, directory)?;
            }
        }
        let mut resolved = BTreeSet::new();
        while resolved.len() < packages.len() {
            let before = resolved.len();
            for (id, (manifest, _)) in &packages {
                if manifest
                    .runtime_dependencies
                    .iter()
                    .all(|dep| resolved.contains(&dep.plugin))
                {
                    resolved.insert(id);
                }
            }
            if before == resolved.len() {
                return Err(io::Error::other(RuntimeDependencyError::Cycle));
            }
        }
        Ok(())
    }

    pub(super) fn check_runtime_removal(
        &self,
        snapshot: &InstalledPlugins,
        id: &str,
    ) -> io::Result<()> {
        for (other, revision) in &snapshot.active {
            if other == id {
                continue;
            }
            let plugin = self.installed(other, revision, snapshot)?;
            if plugin
                .manifest
                .runtime_dependencies
                .iter()
                .any(|dep| dep.plugin == id)
            {
                return Err(io::Error::other(RuntimeDependencyError::RequiredBy {
                    provider: id.into(),
                    consumer: other.clone(),
                }));
            }
        }
        let file = match self.runtime_lease(id) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        match file.try_lock() {
            Ok(()) => Ok(()),
            Err(fs::TryLockError::WouldBlock) => {
                Err(io::Error::other(RuntimeDependencyError::Busy(id.into())))
            }
            Err(fs::TryLockError::Error(error)) => Err(error),
        }
    }
}
