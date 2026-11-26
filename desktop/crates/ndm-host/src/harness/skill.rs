use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    io::{Read, Write},
    path::Path,
};

use ndm_core::{
    Store,
    plugins::{InstalledPlugin, OptionControl, PluginKind, PluginRegistry},
};

const SKILL: &str = include_str!("../../skills/SKILL.md");
const PLUGIN_INDEX: &str = include_str!("../../skills/references/plugins/index.md");
const OWNER: &str = "<!-- Managed by the OnesDM desktop integration. -->";
const LEGACY_OWNER: &str = "<!-- Managed by the NDM Next desktop integration. -->";
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;
const DIRECTORIES: &[&str] = &["plugins", "sdk", "sdk/examples"];
const RESOURCES: &[(&str, &str)] = &[
    (
        "downloads.md",
        include_str!("../../skills/references/downloads.md"),
    ),
    (
        "settings.md",
        include_str!("../../skills/references/settings.md"),
    ),
    (
        "diagnostics.md",
        include_str!("../../skills/references/diagnostics.md"),
    ),
    (
        "sdk/quickstart.md",
        include_str!("../../skills/references/sdk/quickstart.md"),
    ),
    (
        "sdk/README.md",
        include_str!("../../../ndm-plugin-sdk/README.md"),
    ),
    (
        "sdk/examples/file-copy.rs",
        include_str!("../../../ndm-plugin-sdk/examples/file-copy.rs"),
    ),
    (
        "sdk/examples/file-copy.json",
        include_str!("../../../ndm-plugin-sdk/examples/file-copy.json"),
    ),
];

pub(super) fn is_managed(path: &Path) -> io::Result<bool> {
    if !directory_exists(path.parent().expect("Skill parent"))? {
        return Ok(false);
    }
    Ok(owned_document(path)?.is_some())
}

pub(super) fn is_installed(path: &Path) -> io::Result<bool> {
    if !is_managed(path)? {
        return Ok(false);
    }
    let references = path.parent().expect("Skill parent").join("references");
    if !directory_exists(&references)? {
        return Ok(false);
    }
    for directory in DIRECTORIES {
        if !directory_exists(&references.join(directory))? {
            return Ok(false);
        }
    }
    for relative in RESOURCES
        .iter()
        .map(|(name, _)| *name)
        .chain(["plugins/index.md"])
    {
        if owned_document(&references.join(relative))?.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn resource_document(relative: &str, source: &str) -> io::Result<String> {
    // The source tree and the installed bundle both link to the same SDK and examples.
    if relative == "sdk/quickstart.md" {
        return Ok(format!(
            "{OWNER}\n\n{}",
            source.replace("../../../../ndm-plugin-sdk/", "")
        ));
    }
    // Keep both templates usable as native Rust and JSON after installation.
    if relative.ends_with(".rs") {
        return Ok(format!("/*\n{OWNER}\n*/\n{source}"));
    }
    if relative.ends_with(".json") {
        let mut manifest: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(source)?;
        manifest.insert("$comment".into(), OWNER.into());
        return Ok(format!("{}\n", serde_json::to_string_pretty(&manifest)?));
    }
    Ok(format!("{OWNER}\n\n{source}"))
}

pub(super) fn write(path: &Path, profile: &Path) -> io::Result<()> {
    let store = Store::open(&profile.join("tasks.db")).map_err(io::Error::other)?;
    let enabled = store.ai_enabled().map_err(io::Error::other)?;
    let plugins = if enabled {
        PluginRegistry::new(profile).list()?
    } else {
        Vec::new()
    };
    let mut index = format!("{OWNER}\n\n{PLUGIN_INDEX}\n");
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
            "\nAI access is off in OnesDM. Enable it in the app to see available plugins.\n"
        });
    }

    let directory = path.parent().expect("Skill parent");
    fs::create_dir_all(directory.parent().expect("Skills parent"))?;
    ensure_directory(directory)?;
    let references = directory.join("references");
    ensure_directory(&references)?;
    for dir in DIRECTORIES {
        ensure_directory(&references.join(dir))?;
    }
    let plugins = references.join("plugins");
    let index_path = plugins.join("index.md");
    // Check conflicts before updating currently published guides.
    owned_document(path)?;
    owned_document(&index_path)?;
    for (file, _) in RESOURCES {
        owned_document(&references.join(file))?;
    }
    for file in guides.keys() {
        owned_document(&plugins.join(file))?;
    }
    for (file, source) in RESOURCES {
        write_document(&references.join(file), &resource_document(file, source)?)?;
    }
    for (file, guide) in &guides {
        write_document(&plugins.join(file), guide)?;
    }
    write_document(&index_path, &index)?;
    let skill = format!(
        "{}\n<!-- OnesDM {} -->\n",
        SKILL.replace("../../ndm-plugin-sdk/", "references/sdk/"),
        env!("CARGO_PKG_VERSION")
    );
    write_document(path, &skill)?;
    remove_stale_guides(&plugins, &guides)
}

pub(super) fn remove(path: &Path) -> io::Result<()> {
    let directory = path.parent().expect("Skill parent");
    let references = directory.join("references");
    let plugins = references.join("plugins");
    if directory_exists(&references)? {
        // Check each parent before touching descendants, including on partial installs.
        for dir in DIRECTORIES {
            directory_exists(&references.join(dir))?;
        }
        for (relative, _) in RESOURCES {
            let file = references.join(relative);
            if matches!(owned_document(&file), Ok(Some(_))) {
                fs::remove_file(file)?;
            }
        }
        if directory_exists(&plugins)? {
            remove_stale_guides(&plugins, &BTreeMap::new())?;
            let index = plugins.join("index.md");
            if matches!(owned_document(&index), Ok(Some(_))) {
                fs::remove_file(index)?;
            }
        }
        for dir in DIRECTORIES.iter().rev() {
            let _ = fs::remove_dir(references.join(dir));
        }
        let _ = fs::remove_dir(references);
    }
    fs::remove_file(path)?;
    let _ = fs::remove_dir(directory);
    Ok(())
}

fn plugin_guide(plugin: &InstalledPlugin) -> String {
    let manifest = &plugin.manifest;
    let mut guide = format!(
        "{OWNER}\n\n# {}\n\n{}\n\nPlugin ID `{}`. Version `{}`.\n\n",
        one_line(&manifest.name.en),
        one_line(&manifest.description.en),
        manifest.id,
        manifest.version
    );
    if manifest.kind == PluginKind::CharacterProfiles {
        if let Ok(Some(authored)) = plugin.read_skill() {
            guide.push_str(authored.trim());
            guide.push('\n');
        }
        return guide;
    }
    guide.push_str(
        "Use `ndm_list_engines` with the source to check compatibility and obtain the current options. \
         For a batch, pass this engine ID in each applicable `ndm_download` item. \
         Omit option values to use saved settings and manifest defaults. [Download recipes](../downloads.md)\n",
    );
    for (label, rules) in [
        ("Automatic sources", &manifest.inputs),
        ("Explicit selection", &manifest.manual_inputs),
    ] {
        if rules.is_empty() {
            continue;
        }
        writeln!(guide, "\n## {label}\n").unwrap();
        for rule in rules.iter().take(4) {
            write!(
                guide,
                "- Schemes {}",
                preview(rule.schemes.iter().map(String::as_str))
            )
            .unwrap();
            for (label, values) in [
                ("hosts", &rule.hosts),
                ("suffixes", &rule.suffixes),
                ("excluded suffixes", &rule.exclude_suffixes),
            ] {
                if !values.is_empty() {
                    write!(
                        guide,
                        " · {label} {}",
                        preview(values.iter().map(String::as_str))
                    )
                    .unwrap();
                }
            }
            guide.push('\n');
        }
        if rules.len() > 4 {
            writeln!(
                guide,
                "- {} additional rules. Check the actual source with `ndm_list_engines`.",
                rules.len() - 4
            )
            .unwrap();
        }
    }
    writeln!(
        guide,
        "\nThe engine allows up to {} simultaneous tasks.",
        manifest.limits.active_tasks
    )
    .unwrap();
    if let Some(threads) = manifest.limits.download_threads {
        writeln!(guide, "The per-task `download_threads` limit is {threads}.").unwrap();
    }
    if !manifest.options.is_empty() {
        guide.push_str("\n## Task options\n\n");
    }
    for field in &manifest.options {
        let values = match &field.control {
            OptionControl::Boolean => "boolean".into(),
            OptionControl::Integer { min, max } => format!("integer {min}–{max}"),
            OptionControl::Text { max_length } => format!("text, up to {max_length} characters"),
            OptionControl::Choice { choices } => format!(
                "one of {}",
                preview(choices.iter().map(|choice| choice.value.as_str()))
            ),
        };
        writeln!(
            guide,
            "- `{}` · {} ({values})",
            field.key,
            one_line(&field.label.en)
        )
        .unwrap();
    }
    if !manifest.tools.is_empty() {
        guide.push_str("\n## MCP tools\n\n");
    }
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

fn preview<'a>(mut values: impl Iterator<Item = &'a str>) -> String {
    let mut shown = values
        .by_ref()
        .take(6)
        .map(one_line)
        .collect::<Vec<_>>()
        .join(", ");
    if values.next().is_some() {
        shown.push_str(", …");
    }
    shown
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
    let managed = if path.extension().is_some_and(|ext| ext == "json") {
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.get("$comment").and_then(|v| v.as_str()),
                    Some(OWNER | LEGACY_OWNER)
                )
            })
    } else {
        text.lines()
            .any(|line| matches!(line, OWNER | LEGACY_OWNER))
    };
    if text.len() as u64 > MAX_DOCUMENT_BYTES || !managed {
        return Err(conflict());
    }
    Ok(Some(text))
}

fn write_document(path: &Path, text: &str) -> io::Result<()> {
    if text.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(io::Error::other("OnesDM Skill document is too large"));
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
        "an unmanaged OnesDM Skill path exists",
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
