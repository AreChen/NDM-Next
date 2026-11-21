use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    io::{Read, Write},
    path::Path,
};

use ndm_core::{
    Store,
    plugins::{InstalledPlugin, PluginRegistry},
};

use super::Target;

const SKILL: &str = include_str!("../../skills/SKILL.md");
const OWNER: &str = "<!-- Managed by the NDM Next desktop integration. -->";
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;

pub(super) fn is_installed(target: &Target) -> io::Result<bool> {
    let path = target.skill_path();
    if !directory_exists(path.parent().expect("Skill parent"))? {
        return Ok(false);
    }
    Ok(owned_document(&path)?.is_some())
}

pub(super) fn write(target: &Target) -> io::Result<()> {
    let store = Store::open(&target.profile.join("tasks.db")).map_err(io::Error::other)?;
    let enabled = store.ai_enabled().map_err(io::Error::other)?;
    let plugins = if enabled {
        PluginRegistry::new(&target.profile).list()?
    } else {
        Vec::new()
    };
    let mut index = format!("{OWNER}\n\n# NDM Next plugin guides\n");
    let mut guides = BTreeMap::new();
    for plugin in plugins.iter().filter(|plugin| plugin.ai_enabled) {
        let id = &plugin.manifest.id;
        let file = format!("plugin-{id}.md");
        let name = one_line(&plugin.manifest.name.en);
        writeln!(
            index,
            "- [`{id}`]({file}) · {name} {}",
            plugin.manifest.version
        )
        .unwrap();
        guides.insert(file, plugin_guide(plugin));
    }
    if guides.is_empty() {
        index.push_str(if enabled {
            "\nNo installed plugin is enabled for AI access. Use the built-in engine.\n"
        } else {
            "\nAI access is off in NDM Next. Enable it in the app to see available plugins.\n"
        });
    }

    let path = target.skill_path();
    let directory = path.parent().expect("Skill parent");
    fs::create_dir_all(directory.parent().expect("Skills parent"))?;
    ensure_directory(directory)?;
    let references = directory.join("references");
    ensure_directory(&references)?;
    let plugins = references.join("plugins");
    ensure_directory(&plugins)?;
    let index_path = plugins.join("index.md");
    // Check conflicts before updating currently published guides.
    owned_document(&path)?;
    owned_document(&index_path)?;
    for file in guides.keys() {
        owned_document(&plugins.join(file))?;
    }
    for (file, guide) in &guides {
        write_document(&plugins.join(file), guide)?;
    }
    write_document(&index_path, &index)?;
    let skill = format!("{SKILL}\n<!-- NDM Next {} -->\n", env!("CARGO_PKG_VERSION"));
    write_document(&path, &skill)?;
    remove_stale_guides(&plugins, &guides)
}

pub(super) fn remove(target: &Target) -> io::Result<()> {
    let path = target.skill_path();
    let directory = path.parent().expect("Skill parent");
    let references = directory.join("references");
    let plugins = references.join("plugins");
    if directory_exists(&references)? && directory_exists(&plugins)? {
        remove_stale_guides(&plugins, &BTreeMap::new())?;
        let index = plugins.join("index.md");
        if matches!(owned_document(&index), Ok(Some(_))) {
            fs::remove_file(index)?;
        }
        let _ = fs::remove_dir(plugins);
        let _ = fs::remove_dir(references);
    }
    fs::remove_file(&path)?;
    let _ = fs::remove_dir(directory);
    Ok(())
}

fn plugin_guide(plugin: &InstalledPlugin) -> String {
    let manifest = &plugin.manifest;
    let mut guide = format!(
        "{OWNER}\n\n# {}\n\n{}\n\nEngine ID `{}`. Version `{}`.\n\n",
        one_line(&manifest.name.en),
        one_line(&manifest.description.en),
        manifest.id,
        manifest.version
    );
    guide.push_str(
        "Use `ndm_list_engines` with the source to check compatibility and obtain the current options. \
         For a batch, pass this engine ID in each applicable `ndm_download` item. \
         Omit option values to use saved settings and manifest defaults.\n",
    );
    for tool in &manifest.tools {
        writeln!(
            guide,
            "- `{}` — {}",
            tool.mcp_name(&manifest.id),
            one_line(&tool.description.en)
        )
        .unwrap();
    }
    if let Ok(Some(authored)) = plugin.read_skill() {
        guide.push_str("\n## Plugin usage guide\n\n");
        guide.push_str(authored.trim());
        guide.push('\n');
    }
    guide
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn remove_stale_guides(directory: &Path, keep: &BTreeMap<String, String>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file = entry.file_name();
        if file.to_str().is_some_and(|name| {
            name.starts_with("plugin-") && name.ends_with(".md") && !keep.contains_key(name)
        }) && matches!(owned_document(&entry.path()), Ok(Some(_)))
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn directory_exists(path: &Path) -> io::Result<bool> {
    Ok(match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => true,
        Ok(_) => return Err(conflict()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    })
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    if !directory_exists(path)? {
        fs::create_dir(path)?;
    }
    Ok(())
}

fn owned_document(path: &Path) -> io::Result<Option<String>> {
    let metadata = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    if !metadata.is_file() || is_link(&metadata) || metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(conflict());
    }
    let mut text = String::new();
    fs::File::open(path)?
        .take(MAX_DOCUMENT_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_DOCUMENT_BYTES || !text.lines().any(|line| line == OWNER) {
        return Err(conflict());
    }
    Ok(Some(text))
}

fn write_document(path: &Path, text: &str) -> io::Result<()> {
    if text.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(io::Error::other("NDM Next Skill document is too large"));
    }
    let previous = owned_document(path)?;
    if previous.as_deref() == Some(text) {
        return Ok(());
    }
    let mut pending = tempfile::NamedTempFile::new_in(path.parent().expect("document parent"))?;
    pending.write_all(text.as_bytes())?;
    pending.as_file().sync_all()?;
    if previous.is_some() {
        pending.persist(path)
    } else {
        pending.persist_noclobber(path)
    }
    .map_err(io::Error::other)?;
    Ok(())
}

fn conflict() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "an unmanaged NDM Next Skill path exists",
    )
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    metadata.is_symlink()
}
