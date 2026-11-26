//! User-owned download roles. Only the installed character plugin exposes these tools.
use std::{
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use ndm_core::{
    ProxyMode,
    plugins::{PluginKind, PluginRegistry},
};
use ndm_plugin_sdk::{valid_identifier, valid_package_path};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::web::{self, MediaType, UserAgentProfile};

pub const PLUGIN_ID: &str = "character";
const SKILL_FILE: &str = "SKILL.md";
const MARKER: &str = "<!-- OnesDM character data ";
const MAX_BYTES: u64 = 64 * 1024;
const MAX_CHARACTERS: usize = 128;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CharacterDefaults {
    pub destination: Option<PathBuf>,
    pub plugin: Option<String>,
    pub options: Map<String, Value>,
    pub user_agent: Option<String>,
    pub user_agent_profile: Option<UserAgentProfile>,
    pub accept_language: Option<String>,
    pub media_type: Option<MediaType>,
    pub proxy_mode: Option<ProxyMode>,
    pub download_threads: Option<u32>,
    pub show_progress: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub defaults: CharacterDefaults,
}

impl Character {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_id(&self.id) {
            return Err("Use a lowercase character ID with up to 40 letters, digits or hyphens.");
        }
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || self.description.trim().is_empty()
            || self.description.len() > 512
            || self.description.chars().any(char::is_control)
            || self.instructions.len() > 4096
            || self.instructions.contains('\0')
        {
            return Err("Invalid character name, description or instructions.");
        }
        let d = &self.defaults;
        web::headers(
            d.user_agent.as_deref(),
            d.user_agent_profile,
            None,
            d.accept_language.as_deref(),
        )?;
        if d.proxy_mode.is_some_and(|mode| mode != ProxyMode::Inherit)
            && d.plugin.as_deref().is_some_and(|id| id != "builtin")
        {
            return Err(
                "Per-task network routes apply to the built-in engine. Use plugin options for a plugin proxy.",
            );
        }
        if d.destination
            .as_ref()
            .is_some_and(|p| !p.is_absolute() || p.to_string_lossy().chars().any(char::is_control))
            || d.plugin
                .as_deref()
                .is_some_and(|id| !valid_identifier(id) || id == PLUGIN_ID)
            || d.download_threads.is_some_and(|n| !(1..=256).contains(&n))
            || (!d.options.is_empty() && d.plugin.as_deref().is_none_or(|id| id == "builtin"))
            || d.options.len() > 24
            || serde_json::to_vec(&d.options).is_ok_and(|bytes| bytes.len() > 4096)
            || (d.media_type.is_some() && d.plugin.as_deref().is_some_and(|id| id != "builtin"))
        {
            return Err("Invalid character defaults. Plugin options require an explicit engine.");
        }
        Ok(())
    }

    pub fn preview(&self) -> Value {
        let mut defaults = serde_json::to_value(&self.defaults).expect("serializable character");
        defaults.as_object_mut().unwrap().remove("options");
        json!({
            "id":self.id, "name":self.name, "description":self.description,
            "instructions":self.instructions, "defaults":defaults,
            "configured_option_keys":self.defaults.options.keys().collect::<Vec<_>>(),
        })
    }

    fn document(&self) -> io::Result<String> {
        let protected =
            ndm_core::protect_user_data(&serde_json::to_vec(self)?).map_err(io::Error::other)?;
        Ok(format!(
            "---\nname: {}\ndescription: {}\n---\n\n# {}\n\n{}\n\nUse the {} character in ndm_download. Read ndm_get_character for its current defaults.\n\n{MARKER}{} -->\n",
            self.id,
            serde_json::to_string(&self.description)?,
            self.name,
            self.instructions,
            self.id,
            serde_json::to_string(&protected)?,
        ))
    }

    fn from_document(contents: &str, id: &str) -> io::Result<Self> {
        let line = contents
            .lines()
            .rev()
            .find_map(|line| line.strip_prefix(MARKER))
            .and_then(|line| line.strip_suffix(" -->"))
            .ok_or_else(|| io::Error::other("not a managed OnesDM character"))?;
        let protected: Vec<u8> = serde_json::from_str(line)?;
        let bytes = ndm_core::unprotect_user_data(&protected).map_err(io::Error::other)?;
        let character: Self = serde_json::from_slice(&bytes)?;
        if character.id != id {
            return Err(io::Error::other(
                "character ID does not match the Skill path",
            ));
        }
        character.validate().map_err(io::Error::other)?;
        Ok(character)
    }
}

pub fn available(registry: &PluginRegistry) -> bool {
    registry
        .get(PLUGIN_ID)
        .ok()
        .flatten()
        .is_some_and(|plugin| {
            plugin.ai_enabled && plugin.manifest.kind == PluginKind::CharacterProfiles
        })
}

pub fn read(profile: &Path, id: &str) -> io::Result<Character> {
    let directory = character_directory(profile, id, false)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Character not found."))?;
    read_file(&directory.join(SKILL_FILE), id)
}

pub fn list(profile: &Path) -> io::Result<Vec<Character>> {
    let Some(root) = root_directory(profile, false)? else {
        return Ok(Vec::new());
    };
    let mut characters = Vec::new();
    for entry in fs::read_dir(root)? {
        let id = entry?.file_name();
        if let Some(id) = id.to_str().filter(|id| valid_id(id))
            && let Ok(character) = read(profile, id)
        {
            characters.push(character);
        }
    }
    characters.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(characters)
}

pub fn save(
    profile: &Path,
    id: String,
    replace: bool,
    apply: impl FnOnce(&mut Character) -> Result<(), &'static str>,
) -> Result<Character, &'static str> {
    const SAVE_ERROR: &str = "Could not save the character Skill.";
    if !valid_id(&id) {
        return Err("Use a lowercase character ID with up to 40 letters, digits or hyphens.");
    }
    let root = root_directory(profile, true)
        .map_err(|_| SAVE_ERROR)?
        .ok_or(SAVE_ERROR)?;
    let _lock = mutation_lock(&root).map_err(|_| SAVE_ERROR)?;
    let mut character = if replace {
        read(profile, &id).map_err(|_| "Character not found.")?
    } else {
        if list(profile).map_err(|_| SAVE_ERROR)?.len() >= MAX_CHARACTERS {
            return Err("Too many download characters.");
        }
        Character {
            id,
            name: String::new(),
            description: String::new(),
            instructions: String::new(),
            defaults: Default::default(),
        }
    };
    apply(&mut character)?;
    character.validate()?;
    write_skill(profile, &character, replace).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            "Character already exists. Use replace only for a requested update."
        } else {
            SAVE_ERROR
        }
    })?;
    Ok(character)
}

fn write_skill(profile: &Path, character: &Character, replace: bool) -> io::Result<()> {
    let document = character.document()?;
    if document.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("Character Skill is too large."));
    }
    let directory =
        character_directory(profile, &character.id, true)?.expect("created character directory");
    let destination = directory.join(SKILL_FILE);
    let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
    temp.write_all(document.as_bytes())?;
    temp.as_file().sync_all()?;
    if replace {
        temp.persist(destination).map_err(io::Error::other)?;
    } else {
        temp.persist_noclobber(destination)
            .map_err(|error| error.error)?;
    }
    Ok(())
}

pub fn delete(profile: &Path, id: &str) -> io::Result<()> {
    let root = root_directory(profile, false)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Character not found."))?;
    let _lock = mutation_lock(&root)?;
    let directory = character_directory(profile, id, false)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Character not found."))?;
    let file = directory.join(SKILL_FILE);
    read_file(&file, id)?;
    fs::remove_file(file)?;
    let _ = fs::remove_dir(directory);
    Ok(())
}

fn read_file(path: &Path, id: &str) -> io::Result<Character> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || reparse(&metadata) || metadata.len() > MAX_BYTES {
        return Err(io::Error::other("Invalid character Skill file."));
    }
    let mut contents = String::new();
    fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut contents)?;
    if contents.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("Character Skill is too large."));
    }
    Character::from_document(&contents, id)
}

fn valid_id(id: &str) -> bool {
    valid_identifier(id) && valid_package_path(id)
}

fn character_directory(profile: &Path, id: &str, create: bool) -> io::Result<Option<PathBuf>> {
    if !valid_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid character ID.",
        ));
    }
    let Some(root) = root_directory(profile, create)? else {
        return Ok(None);
    };
    let directory = root.join(id);
    Ok(directory_exists(&directory, create)?.then_some(directory))
}

fn root_directory(profile: &Path, create: bool) -> io::Result<Option<PathBuf>> {
    let mut root = profile.to_owned();
    for part in ["plugin-data", PLUGIN_ID, "skills"] {
        root.push(part);
        if !directory_exists(&root, create)? {
            return Ok(None);
        }
    }
    Ok(Some(root))
}

fn directory_exists(path: &Path, create: bool) -> io::Result<bool> {
    if create
        && let Err(error) = fs::create_dir(path)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(error);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !reparse(&metadata) => Ok(true),
        Ok(_) => Err(io::Error::other(
            "Character storage is not a regular directory.",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn mutation_lock(root: &Path) -> io::Result<fs::File> {
    let path = root.join(".characters.lock");
    if fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_file() || reparse(&metadata)) {
        return Err(io::Error::other("Invalid character lock file."));
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_symlink()
    }
}
