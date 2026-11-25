//! DeepSeek Harness loads the global Cordis patch list after each profile.
use std::io;

use yaml_edit::{Mapping, Sequence, YamlFile};

use super::{
    LEGACY, McpStatus, SERVER, Target, config::registration_status, config_conflict, invalid_config,
};

fn entry_id(server: &str) -> &'static str {
    if server == LEGACY {
        "mcp-ndm-next"
    } else {
        "mcp-onesdm-mcp"
    }
}
const CLIENT: &str = "@deepseek-ai/dsh-mcp-client";
const MANAGER: &str = "@deepseek-ai/dsh-mcp-manager";

pub(super) struct PatchFile {
    file: YamlFile,
    patches: Sequence,
}

struct Entry {
    patch_index: usize,
    standalone: bool,
    entries: Sequence,
    entry_index: usize,
    row: Mapping,
    config: Mapping,
}

impl PatchFile {
    pub(super) fn parse(contents: &str) -> io::Result<Self> {
        // An uninitialized home may have an empty or comment-only layer.
        let initial;
        let contents = if contents.lines().all(|line| {
            let line = line.trim();
            line.is_empty() || line.starts_with('#')
        }) {
            let separator = if contents.is_empty() || contents.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            initial = format!("{contents}{separator}[]\n");
            &initial
        } else {
            contents
        };
        let file = contents.parse::<YamlFile>().map_err(|_| invalid_config())?;
        let mut documents = file.documents();
        let patches = documents
            .next()
            .and_then(|document| document.as_sequence())
            .ok_or_else(invalid_config)?;
        if documents.next().is_some() {
            return Err(invalid_config());
        }
        drop(documents);
        Ok(Self { file, patches })
    }

    pub(super) fn contents(&self) -> String {
        self.file.to_string()
    }

    fn entry(&self, server: &str) -> io::Result<Option<Entry>> {
        let mut found = None;
        for (patch_index, item) in self.patches.values().enumerate() {
            let patch = item.as_mapping().ok_or_else(invalid_config)?;
            // A later patch could override a managed row or register a server
            // of the same name with the web/desktop server manager.
            if reserves_server(patch, server)
                || conflicts_in_group(patch, server)
                || opaque_server(patch)
            {
                return Err(config_conflict());
            }
            let Some(entries) = patch.get_sequence("insert") else {
                if patch.contains_key("insert") {
                    return Err(invalid_config());
                }
                continue;
            };
            for (entry_index, item) in entries.values().enumerate() {
                let row = item.as_mapping().ok_or_else(invalid_config)?;
                if manager_has_server(row, server)
                    || conflicts_in_group(row, server)
                    || opaque_server(row)
                {
                    return Err(config_conflict());
                }
                let config = row.get_mapping("config");
                let is_client = string(row, "name").as_deref() == Some(CLIENT);
                let matches_server = config
                    .as_ref()
                    .and_then(|cfg| string(cfg, "serverName"))
                    .as_deref()
                    == Some(server);
                let matches_id = string(row, "id").as_deref() == Some(entry_id(server));
                if !matches_id && !(is_client && matches_server) {
                    continue;
                }
                let config = config.ok_or_else(config_conflict)?;
                if found.is_some()
                    || patch.contains_key("id")
                    || !matches_id
                    || !is_client
                    || !matches_server
                    || string(&config, "transport").as_deref() != Some("stdio")
                    || config.contains_key("url")
                {
                    return Err(config_conflict());
                }
                found = Some(Entry {
                    patch_index,
                    standalone: patch.len() == 1,
                    entries: entries.clone(),
                    entry_index,
                    row: row.clone(),
                    config,
                });
            }
        }
        Ok(found)
    }

    pub(super) fn status(&self, target: &Target, server: &str) -> io::Result<McpStatus> {
        let Some(entry) = self.entry(server)? else {
            return Ok(McpStatus::Missing);
        };
        let args = entry.config.get_sequence("args").and_then(|args| {
            args.values()
                .map(|node| node.as_scalar().map(|arg| arg.as_string()))
                .collect::<Option<Vec<_>>>()
        });
        let disabled = entry
            .row
            .get("disabled")
            .map(|node| {
                node.as_scalar()
                    .and_then(|scalar| scalar.as_bool())
                    .ok_or_else(config_conflict)
            })
            .transpose()?
            .unwrap_or(false);
        registration_status(
            target,
            string(&entry.config, "command").as_deref(),
            args.as_deref(),
            !disabled,
        )
    }

    pub(super) fn rename_legacy(&self, executable: &str) -> io::Result<()> {
        let entry = self.entry(LEGACY)?.ok_or_else(config_conflict)?;
        entry.row.set("id", entry_id(SERVER));
        entry.config.set("serverName", SERVER);
        entry.config.set("command", executable);
        Ok(())
    }

    pub(super) fn register(&self, target: &Target) -> io::Result<()> {
        let executable = target.executable.to_str().ok_or_else(invalid_config)?;
        if let Some(entry) = self.entry(SERVER)? {
            entry.config.set("command", executable);
            if entry.row.contains_key("disabled") {
                entry.row.set("disabled", false);
            }
        } else {
            let config = Mapping::new();
            config.set("serverName", SERVER);
            config.set("transport", "stdio");
            config.set("command", executable);
            let args = Sequence::new_flow();
            for arg in target.launch_arguments()? {
                args.push(arg);
            }
            config.set("args", args);
            let row = Mapping::new();
            row.set("id", entry_id(SERVER));
            row.set("name", CLIENT);
            row.set("config", config);
            let entries = Sequence::new();
            entries.push(row);
            let patch = Mapping::new();
            patch.set("insert", entries);
            self.patches.push(patch);
        }
        Ok(())
    }

    pub(super) fn unregister(&self) -> io::Result<()> {
        if let Some(entry) = self.entry(SERVER)? {
            if entry.standalone && entry.entries.len() == 1 {
                self.patches.remove(entry.patch_index);
            } else if entry.entries.len() == 1 {
                entry.entries.clear();
            } else {
                entry.entries.remove(entry.entry_index);
            }
        }
        Ok(())
    }
}

fn string(mapping: &Mapping, key: &str) -> Option<String> {
    mapping
        .get(key)?
        .as_scalar()
        .map(|scalar| scalar.as_string())
}

fn manager_has_server(row: &Mapping, server: &str) -> bool {
    let name = string(row, "name");
    (name.as_deref() == Some(MANAGER) || name.is_none())
        && row
            .get_mapping("config")
            .and_then(|cfg| cfg.get_mapping("servers"))
            .is_some_and(|servers| servers.contains_key(server))
}

fn reserves_server(row: &Mapping, server: &str) -> bool {
    let name = string(row, "name");
    let config = row.get_mapping("config");
    string(row, "id").as_deref() == Some(entry_id(server))
        || manager_has_server(row, server)
        || config.is_some_and(|cfg| {
            string(&cfg, "serverName").as_deref() == Some(server)
                && (name.as_deref() == Some(CLIENT)
                    || (name.is_none() && cfg.contains_key("transport")))
        })
}

// A user may compute an MCP namespace with !!js or share it through a YAML
// alias. Executing the expression or changing that shared config would be unsafe.
fn opaque_server(row: &Mapping) -> bool {
    let name = string(row, "name");
    if row.contains_key("<<") || row.get("name").is_some_and(|name| name.is_alias()) {
        return true;
    }
    let known_mcp = matches!(name.as_deref(), Some(CLIENT | MANAGER));
    let Some(config) = row.get_mapping("config") else {
        return known_mcp && row.contains_key("config");
    };
    (known_mcp || name.is_none())
        && (config.contains_key("<<")
            || config
                .get("serverName")
                .is_some_and(|value| value.as_scalar().is_none())
            || config.get("servers").is_some_and(|value| value.is_alias()))
}

// A group nests entries in config. A patch can also replace a pre-existing
// group's config without repeating its group flag or plugin name.
fn conflicts_in_group(row: &Mapping, server: &str) -> bool {
    let group = row
        .get("group")
        .and_then(|node| node.as_scalar().and_then(|s| s.as_bool()));
    if group != Some(true) && (group.is_some() || row.contains_key("name")) {
        return false;
    }
    row.get_sequence("config").is_some_and(|entries| {
        entries.values().any(|entry| {
            entry.as_mapping().is_some_and(|row| {
                reserves_server(row, server)
                    || conflicts_in_group(row, server)
                    || opaque_server(row)
            }) || (group == Some(true) && entry.is_alias())
        })
    })
}
