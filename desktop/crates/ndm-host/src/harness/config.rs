use std::{
    fs, io,
    io::{Read, Write as _},
    path::{Path, PathBuf},
};

use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstNode, CstObject, CstObjectProp, CstRootNode},
};
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, value};

use super::{
    Harness, MCP_ARGS, McpStatus, SERVER, Target, config_conflict, deepseek, host_name,
    invalid_config,
};

const MAX_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
const CODEX_SERVERS: &str = "mcp_servers";

enum Document {
    Toml(DocumentMut),
    Json(CstRootNode),
    Cordis(deepseek::PatchFile),
}

pub(super) struct Config {
    path: PathBuf,
    original: Option<String>,
    document: Document,
}

impl Config {
    pub(super) fn open(path: PathBuf, client: Harness) -> io::Result<Self> {
        // A dotfiles symlink should continue to point to the same file.
        let path = match path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => path,
            Err(error) => return Err(error),
        };
        let original = read(&path)?;
        let contents = original.as_deref().unwrap_or("");
        let document = if client == Harness::Codex {
            Document::Toml(contents.parse().map_err(|_| invalid_config())?)
        } else if client == Harness::DeepSeek {
            Document::Cordis(deepseek::PatchFile::parse(contents)?)
        } else {
            let jsonc = client == Harness::OpenCode;
            let options = ParseOptions {
                allow_comments: jsonc,
                allow_trailing_commas: jsonc,
                allow_loose_object_property_names: false,
                allow_missing_commas: false,
                allow_single_quoted_strings: false,
                allow_hexadecimal_numbers: false,
                allow_unary_plus_numbers: false,
                allow_bare_decimal_point_numbers: false,
                allow_non_finite_numbers: false,
                allow_extended_string_escapes: false,
            };
            let root = CstRootNode::parse(contents.trim_start_matches('\u{feff}'), &options)
                .map_err(|_| invalid_config())?;
            if root.value().is_some() && root.object_value().is_none() {
                return Err(invalid_config());
            }
            Document::Json(root)
        };
        Ok(Self {
            path,
            original,
            document,
        })
    }

    pub(super) fn exists(&self) -> bool {
        self.original.is_some()
    }

    pub(super) fn status(&self, target: &Target, client: Harness) -> io::Result<McpStatus> {
        match &self.document {
            Document::Toml(document) => codex_status(document, target),
            Document::Json(document) => json_status(document, target, client),
            Document::Cordis(document) => document.status(target),
        }
    }

    pub(super) fn register(&mut self, target: &Target, client: Harness) -> io::Result<()> {
        let executable = target.executable.to_str().ok_or_else(invalid_config)?;
        let args = target.launch_arguments()?;
        match &mut self.document {
            Document::Toml(document) => {
                let servers = ensure_table(document.as_table_mut(), CODEX_SERVERS)?;
                let entry = ensure_table(servers, SERVER)?;
                entry.insert("command", value(executable));
                entry.insert("args", value(args.into_iter().collect::<Array>()));
                if entry.contains_key("enabled") {
                    entry.insert("enabled", value(true));
                }
            }
            Document::Json(document) => {
                let (servers, v2) = json_servers(document, client, true)?;
                let servers = servers.expect("configured with an object");
                let is_new = property(&servers, SERVER)?.is_none();
                let entry = json_object(&servers, SERVER, true)?.expect("created an entry");
                if client == Harness::OpenCode {
                    let command = std::iter::once(executable)
                        .chain(args)
                        .map(Into::into)
                        .collect();
                    set_json(&entry, "type", "local".into());
                    set_json(&entry, "command", CstInputValue::Array(command));
                    if v2 {
                        if is_new {
                            set_json(&entry, "codemode", false.into());
                        }
                        if entry.get("disabled").is_some() {
                            set_json(&entry, "disabled", false.into());
                        }
                    } else if entry.get("enabled").is_some() {
                        set_json(&entry, "enabled", true.into());
                    }
                } else {
                    set_json(&entry, "type", "stdio".into());
                    set_json(&entry, "command", executable.into());
                    set_json(
                        &entry,
                        "args",
                        CstInputValue::Array(args.into_iter().map(Into::into).collect()),
                    );
                    if entry.get("enabled").is_some() {
                        set_json(&entry, "enabled", true.into());
                    }
                    if is_new && client == Harness::Pi {
                        set_json(&entry, "exposure", "direct".into());
                    }
                }
            }
            Document::Cordis(document) => document.register(target)?,
        }
        Ok(())
    }

    pub(super) fn unregister(&mut self, client: Harness) -> io::Result<()> {
        match &mut self.document {
            Document::Toml(document) => {
                if let Some(servers) = document
                    .get_mut(CODEX_SERVERS)
                    .and_then(Item::as_table_like_mut)
                {
                    servers.remove(SERVER);
                }
            }
            Document::Json(document) => {
                let (servers, _) = json_servers(document, client, false)?;
                if let Some(servers) = servers
                    && let Some(entry) = property(&servers, SERVER)?
                {
                    entry.remove();
                }
            }
            Document::Cordis(document) => document.unregister()?,
        }
        Ok(())
    }

    pub(super) fn save(&self) -> io::Result<Option<String>> {
        let contents = match &self.document {
            Document::Toml(document) => document.to_string(),
            Document::Json(document) => document.to_string(),
            Document::Cordis(document) => document.contents(),
        };
        if self.original.as_deref() == Some(&contents) {
            return Ok(None);
        }
        replace(&self.path, self.original.as_deref(), Some(&contents))?;
        Ok(Some(contents))
    }

    pub(super) fn restore(&self, written: &str) -> io::Result<()> {
        replace(&self.path, Some(written), self.original.as_deref())
    }
}

fn codex_status(document: &DocumentMut, target: &Target) -> io::Result<McpStatus> {
    let Some(servers) = document.get(CODEX_SERVERS) else {
        return Ok(McpStatus::Missing);
    };
    let servers = servers.as_table_like().ok_or_else(invalid_config)?;
    let Some(entry) = servers.get(SERVER) else {
        return Ok(McpStatus::Missing);
    };
    let entry = entry.as_table_like().ok_or_else(config_conflict)?;
    let command = entry.get("command").and_then(Item::as_str);
    let args = entry.get("args").and_then(Item::as_array).and_then(|args| {
        args.iter()
            .map(|arg| arg.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
    });
    let enabled = entry
        .get("enabled")
        .is_none_or(|flag| flag.as_bool() == Some(true));
    if entry.contains_key("url") {
        return Err(config_conflict());
    }
    registration_status(target, command, args.as_deref(), enabled)
}

fn ensure_table<'a>(table: &'a mut dyn TableLike, key: &str) -> io::Result<&'a mut dyn TableLike> {
    if !table.contains_key(key) {
        let mut child = Table::new();
        child.set_implicit(key == CODEX_SERVERS);
        table.insert(key, Item::Table(child));
    }
    table
        .get_mut(key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(invalid_config)
}

fn json_status(document: &CstRootNode, target: &Target, client: Harness) -> io::Result<McpStatus> {
    let (servers, v2) = json_servers(document, client, false)?;
    let Some(servers) = servers else {
        return Ok(McpStatus::Missing);
    };
    // Pi normalizes '-' and '_' to the same server name.
    if client == Harness::Pi && property(&servers, "ndm_next")?.is_some() {
        return Err(config_conflict());
    }
    let Some(entry) = property(&servers, SERVER)? else {
        return Ok(McpStatus::Missing);
    };
    let entry = entry.object_value().ok_or_else(config_conflict)?;
    let transport = property(&entry, "type")?
        .map(|prop| {
            prop.value()
                .and_then(json_string)
                .ok_or_else(invalid_config)
        })
        .transpose()?;
    if client == Harness::OpenCode {
        if transport.as_deref() != Some("local") {
            return Err(config_conflict());
        }
        let command = json_args(&entry, "command")?.unwrap_or_default();
        let (executable, args) = command.split_first().ok_or_else(config_conflict)?;
        let enabled = if v2 {
            !json_flag(&entry, "disabled", false)?
        } else {
            json_flag(&entry, "enabled", true)?
        };
        registration_status(target, Some(executable), Some(args), enabled)
    } else {
        if transport.as_deref().is_some_and(|kind| kind != "stdio")
            || property(&entry, "url")?.is_some()
        {
            return Err(config_conflict());
        }
        let command = property(&entry, "command")?
            .and_then(|prop| prop.value())
            .and_then(json_string);
        let args = json_args(&entry, "args")?;
        registration_status(
            target,
            command.as_deref(),
            args.as_deref(),
            json_flag(&entry, "enabled", true)?,
        )
    }
}

pub(super) fn registration_status(
    target: &Target,
    command: Option<&str>,
    args: Option<&[String]>,
    enabled: bool,
) -> io::Result<McpStatus> {
    let command = command.map(Path::new).ok_or_else(config_conflict)?;
    if command.file_name().is_none_or(|name| name != host_name())
        || !matches!(args, Some([mode, flag, profile]) if mode == MCP_ARGS[0] &&
            flag == MCP_ARGS[1] && Path::new(profile) == target.profile)
    {
        return Err(config_conflict());
    }
    Ok(
        if enabled && command == target.executable && command.is_file() {
            McpStatus::Configured
        } else {
            McpStatus::NeedsRepair
        },
    )
}

fn json_servers(
    document: &CstRootNode,
    client: Harness,
    create: bool,
) -> io::Result<(Option<CstObject>, bool)> {
    let root = if create {
        document.object_value_or_create()
    } else {
        document.object_value()
    };
    let Some(root) = root else {
        return Ok((None, false));
    };
    let key = if client == Harness::OpenCode {
        "mcp"
    } else {
        "mcpServers"
    };
    let servers = json_object(&root, key, create)?;
    if client == Harness::OpenCode
        && let Some(legacy) = &servers
        && let Some(nested) = property(legacy, "servers")?.and_then(|prop| prop.object_value())
        // In V1, "servers" can itself be a server name.
        && nested.get("type").and_then(|prop| prop.value()).and_then(json_string).is_none()
    {
        let has_legacy = property(legacy, SERVER)?.is_some();
        if has_legacy && property(&nested, SERVER)?.is_some() {
            return Err(config_conflict());
        }
        if !has_legacy {
            return Ok((Some(nested), true));
        }
    }
    Ok((servers, false))
}

fn property(object: &CstObject, key: &str) -> io::Result<Option<CstObjectProp>> {
    let mut matches = object
        .properties()
        .into_iter()
        .filter(|prop| prop.decoded_name().as_deref() == Some(key));
    let property = matches.next();
    if matches.next().is_some() {
        return Err(config_conflict());
    }
    Ok(property)
}

fn json_object(object: &CstObject, key: &str, create: bool) -> io::Result<Option<CstObject>> {
    match property(object, key)? {
        Some(prop) => prop.object_value().map(Some).ok_or_else(invalid_config),
        None if create => Ok(object.object_value_or_create(key)),
        None => Ok(None),
    }
}

fn json_string(value: CstNode) -> Option<String> {
    value.as_string_lit()?.decoded_value().ok()
}

fn json_args(object: &CstObject, key: &str) -> io::Result<Option<Vec<String>>> {
    Ok(property(object, key)?
        .and_then(|prop| prop.value())
        .and_then(|value| value.as_array())
        .and_then(|array| array.elements().into_iter().map(json_string).collect()))
}

fn json_flag(object: &CstObject, key: &str, default: bool) -> io::Result<bool> {
    property(object, key)?.map_or(Ok(default), |prop| {
        prop.value()
            .and_then(|value| value.as_boolean_lit())
            .map(|flag| flag.value())
            .ok_or_else(invalid_config)
    })
}

fn set_json(object: &CstObject, key: &str, value: CstInputValue) {
    if let Some(property) = object.get(key) {
        property.set_value(value);
    } else {
        object.append(key, value);
    }
}

fn read(path: &Path) -> io::Result<Option<String>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Err(invalid_config()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(invalid_config());
    }
    let mut content = String::new();
    fs::File::open(path)?
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut content)?;
    if content.len() as u64 > MAX_CONFIG_BYTES {
        return Err(invalid_config());
    }
    Ok(Some(content))
}

fn replace(path: &Path, expected: Option<&str>, contents: Option<&str>) -> io::Result<()> {
    if read(path)?.as_deref() != expected {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "AI client configuration changed",
        ));
    }
    let Some(contents) = contents else {
        return fs::remove_file(path);
    };
    let directory = path.parent().ok_or_else(invalid_config)?;
    fs::create_dir_all(directory)?;
    let mut staged = tempfile::NamedTempFile::new_in(directory)?;
    staged.write_all(contents.as_bytes())?;
    if expected.is_some() {
        staged
            .as_file()
            .set_permissions(fs::metadata(path)?.permissions())?;
    }
    staged.as_file().sync_all()?;
    if read(path)?.as_deref() != expected {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "AI client configuration changed",
        ));
    }
    if expected.is_some() {
        staged.persist(path)
    } else {
        staged.persist_noclobber(path)
    }
    .map_err(|error| error.error)?;
    Ok(())
}
