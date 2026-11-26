//! Local stdio transport for the optional AI client integrations.
use std::{
    io::{self, BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use ndm_core::{
    BatchLimit, DownloadFailure, NewTask, ProxyMode, Store, Task, TaskCommand, TaskId, TaskOptions,
    TaskPolicy, TaskState,
    plugins::{InstalledPlugin, PluginRegistry, built_in_source, parse_source},
};
use ndm_host::{
    characters::{self, CharacterDefaults},
    web::{self, MediaType, UserAgentProfile},
};
use ndm_plugin_sdk::{read_message, write_message};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const PROTOCOL: &str = "2025-11-25";
const SOURCE_SCHEMA: &str = "An HTTP(S), FTP, FTPS, SFTP, or installed plugin source. For local files, pass an absolute path.";
const TOOL_REFRESH: Duration = Duration::from_secs(1);

pub fn run(profile: PathBuf) -> io::Result<()> {
    let server = Server {
        store: Store::open(&profile.join("tasks.db")).map_err(io::Error::other)?,
        registry: PluginRegistry::new(&profile),
        profile,
    };
    let (send, receive) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin());
        while let Ok(Some(message)) = read_message::<Value>(&mut input) {
            if send.send(message).is_err() {
                break;
            }
        }
    });
    let mut output = io::stdout().lock();
    let mut initialized = false;
    let mut subscribed = false;
    let mut tools = Value::Null;
    let mut next_refresh = Instant::now() + TOOL_REFRESH;
    loop {
        let now = Instant::now();
        if now >= next_refresh {
            next_refresh = now + TOOL_REFRESH;
            if subscribed {
                let next = json!(server.tools());
                if tools != next {
                    tools = next;
                    write_message(
                        &mut output,
                        &json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
                    )?;
                }
            }
        }
        match receive.recv_timeout(next_refresh.saturating_duration_since(Instant::now())) {
            Ok(request) => {
                let method = request.get("method").and_then(Value::as_str);
                let id = request.get("id").cloned();
                let params = request.get("params").unwrap_or(&Value::Null);
                if method == Some("notifications/initialized") && initialized {
                    subscribed = true;
                    tools = json!(server.tools());
                }
                let Some(id) = id else {
                    continue;
                };
                let result = match method {
                    Some("initialize") => {
                        initialized = true;
                        let offered = params["protocolVersion"].as_str().unwrap_or(PROTOCOL);
                        let version = match offered {
                            "2024-11-05" | "2025-03-26" | "2025-06-18" | PROTOCOL => offered,
                            _ => PROTOCOL,
                        };
                        Ok(json!({
                            "protocolVersion":version, "capabilities":{"tools":{"listChanged":true}},
                            "serverInfo":{"name":"onesdm-mcp","title":"OnesDM MCP","version":env!("CARGO_PKG_VERSION")},
                            "instructions":"When the user asks to download, fetch, or save files, models or media, use ndm_download to start the requested transfers in OnesDM. Use ndm_list_engines for a media page or plugin source and ndm_probe_streams for a mobile media page with an unknown playlist. Track returned task IDs via ndm_task_status."
                        }))
                    }
                    Some("ping") => Ok(json!({})),
                    _ if !initialized => Err((-32002, "MCP session is not initialized")),
                    Some("tools/list") => {
                        tools = json!(server.tools());
                        Ok(json!({"tools": tools}))
                    }
                    Some("tools/call") => Ok(tool_result(server.call(params))),
                    _ => Err((-32601, "Method not found")),
                };
                reply(&mut output, id, result)?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn reply(
    output: &mut impl Write,
    id: Value,
    result: Result<Value, (i32, &'static str)>,
) -> io::Result<()> {
    let message = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err((code, message)) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
        }
    };
    write_message(output, &message)
}

fn tool_result(result: Result<Value, &'static str>) -> Value {
    match result {
        Ok(value) => {
            json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value})
        }
        Err(message) => json!({"content":[{"type":"text","text":message}],"isError":true}),
    }
}

struct Server {
    store: Store,
    registry: PluginRegistry,
    profile: PathBuf,
}

impl Server {
    fn tools(&self) -> Vec<Value> {
        if !self.store.ai_enabled().unwrap_or(false) {
            return Vec::new();
        }
        let characters = characters::available(&self.registry);
        let task_id = json!({"task_id":{"type":"string","format":"uuid"}});
        let mut tools = vec![
            tool(
                "ndm_download",
                "Start the user's requested downloads in OnesDM and return task IDs for tracking. Use for files, models, known HLS/DASH streams and installed engines. Omit destination for the application's download directory.",
                json!({
                    "items":{"type":"array","minItems":1,"maxItems":128,"items":object(item_fields(characters), &["source"])},
                    "destination":{"type":"string","description":"Absolute directory"},
                    "show_progress":{"type":"boolean","default":false,"description":"Open an independent progress window per item. Omit for quiet downloads in the main list."},
                    "batch_active_tasks":task_limit(),
                    "download_threads":thread_limit()
                }),
                &["items"],
            ),
            tool(
                "ndm_list_tasks",
                "List newest tasks with optional state filtering. Pass next_cursor back as cursor for the next page.",
                json!({
                    "limit":{"type":"integer","minimum":1,"maximum":128},
                    "state":{"type":"string","enum":["prepared","queued","running","paused","completed","failed"]},
                    "cursor":{"type":"integer","minimum":1}
                }),
                &[],
            ),
            tool(
                "ndm_list_engines",
                "Discover download engines available to AI, their options and tool names. Pass a source to check matches and the automatic selection without contacting the source.",
                json!({"source":{"type":"string","description":SOURCE_SCHEMA}}),
                &[],
            ),
            tool(
                "ndm_probe_streams",
                "Find HLS/DASH links in a public page or JSON response with a single bounded GET. Supports phone User-Agent profiles. Use when a mobile media page does not provide a known stream URL.",
                probe_fields(),
                &["source"],
            ),
            tool(
                "ndm_task_status",
                "Look up a task, including recent throughput, idle time and redacted HTTP diagnostics when available.",
                task_id.clone(),
                &["task_id"],
            ),
            tool(
                "ndm_progress_window",
                "Request to show or close one task’s independent progress window. Closing leaves the download running.",
                json!({"task_id":{"type":"string","format":"uuid"},"visible":{"type":"boolean"}}),
                &["task_id", "visible"],
            ),
            tool(
                "ndm_pause_task",
                "Request that OnesDM pause a task and save resumable data.",
                task_id.clone(),
                &["task_id"],
            ),
            tool(
                "ndm_resume_task",
                "Resume or retry an OnesDM task.",
                task_id,
                &["task_id"],
            ),
            tool(
                "ndm_network_routes",
                "Read the built-in HTTP(S) download route default and configured TUN direct exit type. Does not probe VPN state or reveal addresses.",
                json!({}),
                &[],
            ),
            tool(
                "ndm_get_settings",
                "Read current desktop preferences, download limits and whether a saved proxy is available. Credentials are never returned.",
                json!({}),
                &[],
            ),
            tool(
                "ndm_set_settings",
                "Change only the desktop-wide preferences the user requested. Returns the saved settings. Existing transfers retain their options. For concurrency use ndm_set_download_limits.",
                json!({
                    "speed_limit_kib_s":{"type":"integer","minimum":0,"maximum":1000000,"description":"KiB/s; zero is unlimited"},
                    "user_agent":{"type":"string","maxLength":1024,"description":"Empty restores the default Chrome UA"},
                    "download_directory":{"type":"string","description":"Absolute folder for new tasks. Empty restores the system downloads folder."},
                    "cache_directory":{"type":"string","description":"Absolute transfer cache directory. Empty restores the default; existing partial files can resume in their previous cache."},
                    "cache_retention_days":{"type":"integer","minimum":0,"maximum":3650,"description":"Zero disables automatic cleanup"},
                    "auto_resume":{"type":"boolean"},"auto_update":{"type":"boolean","description":"Automatically check for updates"},"ui_animations":{"type":"boolean"},
                    "theme":{"type":"string","enum":["system","light","dark"]},"language":{"type":"string","enum":ndm_core::UI_LANGUAGES},
                    "color_scheme":{"type":"string","enum":ndm_core::COLOR_SCHEMES},
                    "theme_colors":color_fields(&ndm_core::THEME_COLOR_KEYS,"Set selected interface colors. Null restores the default primary or automatic companion color. Selects Custom unless color_scheme is specified."),
                    "download_colors":color_fields(&ndm_core::DOWNLOAD_COLOR_KEYS,"Set selected task state colors across all themes. Null restores the default for that state."),
                    "start_at_login":{"type":"boolean"},
                    "sidebar_collapsed":{"type":"boolean"},
                    "sidebar_width":{"type":"integer","minimum":160,"maximum":320},
                    "download_route":{"type":"string","enum":["inherit","system_route","tun_direct","direct"]},
                    "tun_direct_exit":{"type":"string","description":"Local DIRECT HTTP port or Linux/macOS interface name. Empty clears the exit."},
                    "tun_direct_port":{"type":"integer","minimum":0,"maximum":65535,"description":"Port of the user's local HTTP DIRECT listener. Zero clears the exit. Use instead of tun_direct_exit."},
                    "proxy":object(json!({"url":{"type":"string","description":"HTTP(S) proxy URL. Empty removes the saved proxy."},"username":{"type":"string"},"password":{"type":"string"}}), &["url"]),
                    "use_saved_proxy":{"type":"boolean","description":"Enable an already configured application proxy for new downloads"}
                }),
                &[],
            ),
            tool(
                "ndm_get_download_limits",
                "Read the application-wide task limit and default thread count.",
                json!({}),
                &[],
            ),
            tool(
                "ndm_set_download_limits",
                "Persist application-wide task and thread defaults. Use only when the user asks to change global settings.",
                json!({"active_tasks":task_limit(),"download_threads":thread_limit()}),
                &[],
            ),
        ];
        if characters {
            let id = json!({"id":{"type":"string","description":"Exact saved character ID"}});
            tools.extend([
                tool("ndm_list_characters", "Find saved download characters before selecting one for a user request.", json!({}), &[]),
                tool("ndm_get_character", "Read the requested character's Skill instructions and reusable defaults.", id.clone(), &["id"]),
                tool("ndm_save_character", "Create a reusable download character only when requested. Set replace true to patch an existing character; unspecified fields are preserved.", character_fields(), &["id"]),
                tool("ndm_delete_character", "Delete the named character Skill only when the user explicitly asks to delete it.", id, &["id"]),
            ]);
        }
        for plugin in self
            .registry
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.ai_enabled)
        {
            for action in &plugin.manifest.tools {
                let mut fields = item_fields(characters);
                let fields_object = fields.as_object_mut().expect("tool fields are an object");
                for key in ["plugin", "proxy_mode", "media_type"] {
                    fields_object.remove(key);
                }
                fields["options"] = plugin.option_schema(action);
                if let Some(limit) = plugin.manifest.limits.download_threads {
                    fields["download_threads"]["maximum"] = json!(limit);
                } else {
                    fields
                        .as_object_mut()
                        .expect("tool fields are an object")
                        .remove("download_threads");
                }
                tools.push(tool(
                    &action.mcp_name(&plugin.manifest.id),
                    &action.description.en,
                    fields,
                    &["source"],
                ));
            }
        }
        tools
    }

    fn call(&self, params: &Value) -> Result<Value, &'static str> {
        if !self.store.ai_enabled().unwrap_or(false) {
            return Err("Enable AI integration in OnesDM first.");
        }
        let name = params["name"].as_str().ok_or("Missing tool name.")?;
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        match name {
            "ndm_download" => self.download(parse_arguments(args)?),
            "ndm_probe_streams" => web::probe_streams(parse_arguments(args)?),
            "ndm_list_characters"
            | "ndm_get_character"
            | "ndm_save_character"
            | "ndm_delete_character" => self.character_action(name, args),
            "ndm_list_engines" => self.engines(parse_arguments(args)?),
            "ndm_list_tasks" => {
                let args: List = parse_arguments(args)?;
                if args.cursor.is_some_and(|cursor| cursor < 1) {
                    return Err("Invalid task cursor.");
                }
                let (tasks, next_cursor) = self
                    .store
                    .recent_tasks(args.state, args.cursor, args.limit.unwrap_or(32))
                    .map_err(|_| "Could not read tasks.")?;
                let tasks: Vec<_> = tasks.iter().map(task_json).collect();
                Ok(json!({"tasks":tasks,"next_cursor":next_cursor}))
            }
            "ndm_task_status" => {
                let args: TaskArgument = parse_arguments(args)?;
                self.store
                    .get(args.task_id)
                    .map_err(|_| "Could not read task.")?
                    .as_ref()
                    .map(task_json)
                    .ok_or("Task not found.")
            }
            "ndm_progress_window" => {
                let args: WindowArgument = parse_arguments(args)?;
                if args.visible {
                    self.ensure_desktop()?;
                }
                self.store
                    .request_progress_window(args.task_id, args.visible)
                    .map_err(|_| "Task not found or unavailable.")?;
                self.wake();
                Ok(json!({"task_id":args.task_id, "visible":args.visible, "requested":true}))
            }
            "ndm_pause_task" | "ndm_resume_task" => {
                let args: TaskArgument = parse_arguments(args)?;
                self.ensure_desktop()?;
                let command = if name == "ndm_pause_task" {
                    TaskCommand::Pause
                } else {
                    TaskCommand::Resume
                };
                self.store
                    .request_task_command(args.task_id, command)
                    .map_err(|_| "Task cannot accept that action.")?;
                self.wake();
                Ok(json!({"task_id":args.task_id,"requested":true}))
            }
            "ndm_network_routes" => {
                let prefs = super::read_preferences(&self.profile);
                let exit = prefs.as_ref().and_then(|p| p.tun_egress.as_ref());
                Ok(
                    json!({"default":prefs.as_ref().map(|p| p.download_route).unwrap_or_default(),
                    "tun_direct_exit":exit.map(|e| e.route()),
                    "interface_binding_supported":cfg!(any(target_os="linux",target_os="macos"))}),
                )
            }
            "ndm_get_settings" => {
                let args: ndm_core::SettingsPatch = parse_arguments(args)?;
                if args != Default::default() {
                    return Err("Reading settings takes no arguments.");
                }
                self.desktop_settings(None)
            }
            "ndm_set_settings" => {
                let patch: ndm_core::SettingsPatch = parse_arguments(args)?;
                if patch == Default::default() {
                    return Err("Specify at least one desktop preference.");
                }
                patch.validate()?;
                self.desktop_settings(Some(patch))
            }
            "ndm_get_download_limits" => {
                let limits = self
                    .store
                    .download_limits()
                    .map_err(|_| "Could not read download limits.")?
                    .unwrap_or_else(|| super::configured_limits(&self.profile));
                Ok(json!(limits))
            }
            "ndm_set_download_limits" => {
                let args: Limits = parse_arguments(args)?;
                if args.active_tasks.is_none() && args.download_threads.is_none() {
                    return Err("Specify at least one global limit.");
                }
                let limits = self
                    .store
                    .change_download_limits(
                        args.active_tasks,
                        args.download_threads,
                        super::configured_limits(&self.profile),
                    )
                    .map_err(|_| "Invalid or unavailable download limits.")?;
                let _ = ndm_core::wake_desktop(&self.profile);
                Ok(json!(limits))
            }
            _ => {
                let mut item: Item = parse_arguments(args)?;
                for plugin in self
                    .registry
                    .list()
                    .map_err(|_| "Could not read installed plugins.")?
                    .into_iter()
                    .filter(|plugin| plugin.ai_enabled)
                {
                    for action in &plugin.manifest.tools {
                        if action.mcp_name(&plugin.manifest.id) != name {
                            continue;
                        }
                        item.plugin = Some(plugin.manifest.id.clone());
                        let mut values = action.defaults.clone();
                        values.extend(item.options);
                        item.options = values;
                        return self.download(Batch {
                            items: vec![item],
                            ..Batch::default()
                        });
                    }
                }
                Err("Tool is not available. Refresh the tool list.")
            }
        }
    }

    fn character_action(&self, name: &str, args: Value) -> Result<Value, &'static str> {
        if !characters::available(&self.registry) {
            return Err("Install and enable AI access for the Download Characters plugin.");
        }
        match name {
            "ndm_list_characters" => {
                if args.as_object().is_none_or(|args| !args.is_empty()) {
                    return Err("Listing characters takes no arguments.");
                }
                let records =
                    characters::list(&self.profile).map_err(|_| "Could not list characters.")?;
                let list: Vec<_> = records.into_iter().map(|character| json!({
                    "id":character.id, "name":character.name, "description":character.description
                })).collect();
                Ok(json!({"characters":list}))
            }
            "ndm_save_character" => {
                let args: SaveCharacter = parse_arguments(args)?;
                let character =
                    characters::save(&self.profile, args.id, args.replace, |character| {
                        if let Some(name) = args.name {
                            character.name = name;
                        }
                        if let Some(description) = args.description {
                            character.description = description;
                        }
                        if let Some(instructions) = args.instructions {
                            character.instructions = instructions;
                        }
                        if let Some(mut patch) = args.defaults {
                            let mut previous = serde_json::to_value(&character.defaults)
                                .map_err(|_| "Invalid character defaults.")?;
                            let previous = previous
                                .as_object_mut()
                                .expect("character defaults are an object");
                            if patch
                                .get("plugin")
                                .is_some_and(|plugin| Some(plugin) != previous.get("plugin"))
                            {
                                previous.remove("options");
                            }
                            if let Some(options) = patch.remove("options") {
                                if options.is_null() {
                                    previous.remove("options");
                                } else if let Value::Object(values) = options {
                                    let entry = previous
                                        .entry("options")
                                        .or_insert_with(|| json!({}))
                                        .as_object_mut()
                                        .expect("options are an object");
                                    for (key, value) in values {
                                        if value.is_null() {
                                            entry.remove(&key);
                                        } else {
                                            entry.insert(key, value);
                                        }
                                    }
                                } else {
                                    return Err("Character plugin options must be an object.");
                                }
                            }
                            previous.extend(patch);
                            character.defaults =
                                parse_arguments(Value::Object(std::mem::take(previous)))?;
                        }
                        Ok(())
                    })?;
                Ok(character.preview())
            }
            "ndm_get_character" | "ndm_delete_character" => {
                let args: CharacterArgument = parse_arguments(args)?;
                if name == "ndm_get_character" {
                    Ok(characters::read(&self.profile, &args.id)
                        .map_err(|_| "Character not found.")?
                        .preview())
                } else {
                    characters::delete(&self.profile, &args.id)
                        .map_err(|_| "Could not delete that character Skill.")?;
                    Ok(json!({"id":args.id, "deleted":true}))
                }
            }
            _ => unreachable!("registered character tool"),
        }
    }

    fn engines(&self, query: EngineQuery) -> Result<Value, &'static str> {
        let source = query
            .source
            .as_deref()
            .map(parse_source)
            .transpose()
            .map_err(|_| "Invalid download source.")?;
        let builtin = source.as_ref().map(built_in_source);
        let candidates: Vec<(Option<bool>, InstalledPlugin)> = if let Some(source) = &source {
            self.registry
                .ai_candidates(source)
                .map_err(|_| "Could not read installed plugins.")?
                .into_iter()
                .map(|(automatic, plugin)| (Some(automatic), plugin))
                .collect()
        } else {
            self.registry
                .list()
                .map_err(|_| "Could not read installed plugins.")?
                .into_iter()
                .filter(|plugin| plugin.ai_enabled)
                .map(|plugin| (None, plugin))
                .collect()
        };
        let automatic = candidates
            .first()
            .filter(|(automatic, _)| *automatic == Some(true))
            .map(|(_, plugin)| plugin.manifest.id.as_str())
            .or_else(|| (builtin == Some(true)).then_some("builtin"));
        let plugins: Vec<_> = candidates
            .iter()
            .map(|(automatic, plugin)| {
                let manifest = &plugin.manifest;
                let tools: Vec<_> = manifest
                    .tools
                    .iter()
                    .map(|tool| tool.mcp_name(&manifest.id))
                    .collect();
                json!({
                    "id":manifest.id,
                    "name":manifest.name.en,
                    "description":manifest.description.en,
                    "version":manifest.version,
                    "automatic_match":automatic,
                    "tools":tools,
                    "options":plugin.default_option_schema(),
                    "limits":manifest.limits
                })
            })
            .collect();
        Ok(json!({
            "automatic_engine":automatic,
            "builtin":{"accepts_source":builtin,"protocols":["HTTP(S)","FTP(S)","SFTP","HLS","static DASH"]},
            "plugins":plugins
        }))
    }

    fn download(&self, batch: Batch) -> Result<Value, &'static str> {
        if !(1..=128).contains(&batch.items.len())
            || batch
                .batch_active_tasks
                .is_some_and(|limit| !(1..=32).contains(&limit))
            || batch
                .download_threads
                .is_some_and(|limit| !(1..=256).contains(&limit))
        {
            return Err("Invalid batch size or download limits.");
        }
        let batch_limit = batch.batch_active_tasks.map(|active_tasks| BatchLimit {
            id: TaskId::new_v4(),
            active_tasks,
        });
        let directory = batch
            .destination
            .unwrap_or_else(|| super::configured_downloads(&self.profile));
        let mut tasks = Vec::with_capacity(batch.items.len());
        let tun_configured = super::read_preferences(&self.profile)
            .and_then(|p| p.tun_egress)
            .is_some();
        let mut characters = std::collections::BTreeMap::new();
        for mut item in batch.items {
            if let Some(id) = item.character.take() {
                if !characters::available(&self.registry) {
                    return Err("Install and enable AI access for the Download Characters plugin.");
                }
                let defaults = match characters.entry(id) {
                    std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        let character = characters::read(&self.profile, entry.key())
                            .map_err(|_| "Download character not found.")?;
                        entry.insert(character.defaults)
                    }
                };
                item.apply_character(defaults.clone());
            }
            let source = parse_source(&item.source).map_err(|_| "Invalid download source.")?;
            let web_source = matches!(source.scheme(), "http" | "https");
            let explicit_builtin = item.plugin.as_deref() == Some("builtin");
            let options = if item.media_type.is_some() || explicit_builtin {
                if !built_in_source(&source)
                    || (item.media_type.is_some() && !web_source)
                    || (item.plugin.is_some() && !explicit_builtin)
                    || !item.options.is_empty()
                {
                    return Err(
                        "Explicit HLS/DASH requires a built-in HTTP(S) source and no plugin options.",
                    );
                }
                item.media_type.map(MediaType::task_options)
            } else {
                self.registry
                    .ai_task_options(&source, item.plugin.as_deref(), item.options)
                    .map_err(|_| "Plugin or plugin options do not match the source.")?
            };
            if options.is_none() && !built_in_source(&source) {
                return Err("No installed download engine handles this source.");
            }
            let proxy_mode = item.proxy_mode.unwrap_or_default();
            if proxy_mode != ProxyMode::Inherit
                && (!web_source || matches!(options, Some(TaskOptions::Plugin(_))))
            {
                return Err("Per-task routing applies to built-in HTTP(S) downloads.");
            }
            if proxy_mode == ProxyMode::TunDirect && !tun_configured {
                return Err(
                    "Set the user's DIRECT HTTP listener port with ndm_set_settings before using tun_direct.",
                );
            }
            let headers = web::headers(
                item.user_agent.as_deref(),
                item.user_agent_profile,
                item.referer.as_deref(),
                item.accept_language.as_deref(),
            )?;
            if !headers.is_empty() && !web_source {
                return Err("User-Agent, Referer and Accept-Language require an HTTP(S) source.");
            }
            let destination = item.destination.unwrap_or_else(|| directory.clone());
            let threads = item.download_threads.or(batch.download_threads);
            if !destination.is_absolute() || threads.is_some_and(|n| !(1..=256).contains(&n)) {
                return Err(
                    "Destination must be absolute and thread count must be between 1 and 256.",
                );
            }
            let request = NewTask {
                url: source.into(),
                destination,
                suggested_filename: None,
                source_url: None,
                expected_size: None,
                headers,
            };
            tasks.push((
                TaskId::new_v4(),
                request,
                options.unwrap_or(TaskOptions::Auto),
                TaskPolicy {
                    batch: batch_limit,
                    download_threads: threads,
                    proxy_mode,
                    show_progress: Some(item.show_progress.unwrap_or(batch.show_progress)),
                },
            ));
        }
        self.ensure_desktop()?;
        self.store
            .enqueue(&tasks)
            .map_err(|_| "Could not queue the download batch.")?;
        self.wake();
        Ok(json!({
            "task_ids":tasks.iter().map(|(id, _, _, _)| id).collect::<Vec<_>>(),
            "batch_id":batch_limit.map(|batch| batch.id)
        }))
    }

    fn desktop_settings(
        &self,
        patch: Option<ndm_core::SettingsPatch>,
    ) -> Result<Value, &'static str> {
        self.ensure_desktop()?;
        if let Some(patch) = &patch {
            for (folder, error) in [
                (
                    patch.download_directory.as_ref(),
                    "Download directory is not writable.",
                ),
                (
                    patch.cache_directory.as_ref(),
                    "Cache directory is not writable.",
                ),
            ] {
                if let Some(folder) = folder.filter(|s| !s.is_empty()) {
                    std::fs::create_dir_all(folder)
                        .and_then(|()| tempfile::tempfile_in(folder))
                        .map_err(|_| error)?;
                }
            }
        }
        let id = self
            .store
            .request_settings(patch.as_ref())
            .map_err(|_| "Could not submit settings request.")?;
        self.wake();
        let start = Instant::now();
        let result = loop {
            match self.store.settings_reply(id) {
                Ok(Some(reply)) if reply.get("error").is_some() => {
                    break Err(match reply["error"].as_str() {
                        Some("proxy") => "Configure a saved proxy in OnesDM first.",
                        Some("tun") => "Set a TUN direct exit before selecting that route.",
                        Some("disabled") => "AI integration is disabled.",
                        Some("startup") => {
                            "OnesDM could not update the operating system's startup setting."
                        }
                        Some("busy") => {
                            "Storage settings are busy. Retry after the operation finishes."
                        }
                        _ => "OnesDM could not apply the preferences.",
                    });
                }
                Ok(Some(reply)) => break Ok(reply),
                Err(_) => break Err("Could not read desktop settings reply."),
                _ if start.elapsed() >= Duration::from_secs(15) => {
                    break Err(
                        "Desktop did not acknowledge the request. Read settings before retrying.",
                    );
                }
                _ => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        let _ = self.store.forget_settings_request(id);
        result
    }

    fn ensure_desktop(&self) -> Result<(), &'static str> {
        if ndm_core::wait_for_desktop_ready(&self.profile, Duration::ZERO).unwrap_or(false) {
            return Ok(());
        }
        if !ndm_core::desktop_is_running(&self.profile).unwrap_or(false) {
            let executable = std::env::current_exe()
                .map_err(|_| "Desktop executable is unavailable.")?
                .with_file_name(if cfg!(windows) {
                    "ndm-desktop.exe"
                } else {
                    "ndm-desktop"
                });
            let mut child = Command::new(executable)
                .args(["--handoff", "--background"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| "Could not launch OnesDM.")?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        if ndm_core::wait_for_desktop_ready(&self.profile, Duration::from_secs(20)).unwrap_or(false)
        {
            Ok(())
        } else {
            Err("OnesDM did not become ready.")
        }
    }

    fn wake(&self) {
        // The write is durable. An app exit racing the wake will pick up the queue on restart.
        let _ = ndm_core::wake_desktop(&self.profile);
    }
}

fn parse_arguments<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, &'static str> {
    serde_json::from_value(value).map_err(|_| "Invalid tool arguments.")
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    #[serde(default)]
    show_progress: bool,
    items: Vec<Item>,
    destination: Option<PathBuf>,
    batch_active_tasks: Option<u32>,
    download_threads: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    show_progress: Option<bool>,
    source: String,
    character: Option<String>,
    destination: Option<PathBuf>,
    plugin: Option<String>,
    download_threads: Option<u32>,
    proxy_mode: Option<ProxyMode>,
    user_agent: Option<String>,
    user_agent_profile: Option<UserAgentProfile>,
    referer: Option<String>,
    accept_language: Option<String>,
    media_type: Option<MediaType>,
    #[serde(default)]
    options: Map<String, Value>,
}

impl Item {
    fn apply_character(&mut self, mut defaults: CharacterDefaults) {
        self.destination = self.destination.take().or(defaults.destination);
        self.download_threads = self.download_threads.or(defaults.download_threads);
        self.proxy_mode = self.proxy_mode.or(defaults.proxy_mode);
        self.show_progress = self.show_progress.or(defaults.show_progress);
        self.accept_language = self.accept_language.take().or(defaults.accept_language);
        if self.user_agent.is_none() && self.user_agent_profile.is_none() {
            self.user_agent = defaults.user_agent;
            self.user_agent_profile = defaults.user_agent_profile;
        }
        let explicit_media_overrides_plugin = self.media_type.is_some()
            && defaults.plugin.as_deref().is_some_and(|id| id != "builtin");
        if !explicit_media_overrides_plugin
            && (self.plugin.is_none() || self.plugin == defaults.plugin)
        {
            self.plugin = self.plugin.take().or(defaults.plugin);
            self.media_type = self.media_type.or(defaults.media_type);
            defaults.options.append(&mut self.options);
            self.options = defaults.options;
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterArgument {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveCharacter {
    id: String,
    name: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
    defaults: Option<Map<String, Value>>,
    #[serde(default)]
    replace: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowArgument {
    task_id: TaskId,
    visible: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    active_tasks: Option<u32>,
    download_threads: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    limit: Option<usize>,
    state: Option<TaskState>,
    cursor: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineQuery {
    source: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskArgument {
    task_id: TaskId,
}

fn task_json(task: &Task) -> Value {
    let engine = match &task.options {
        TaskOptions::Plugin(plugin) => plugin.id.as_str(),
        _ => "builtin",
    };
    let name = task
        .output_path
        .as_deref()
        .and_then(std::path::Path::file_name)
        .map(|name| name.to_string_lossy())
        .or_else(|| task.engine_title.as_deref().map(Into::into))
        .or_else(|| task.request.suggested_filename.as_deref().map(Into::into))
        .or_else(|| {
            if matches!(task.options, TaskOptions::Plugin(_)) {
                return None;
            }
            ndm_core::http_filename_hint(&reqwest::Url::parse(&task.request.url).ok()?)
                .map(Into::into)
        });
    let http = task
        .progress
        .as_ref()
        .and_then(|progress| progress.http.as_ref());
    let error = (task.state == TaskState::Failed).then(|| {
        let code = http.and_then(|details| details.failure).unwrap_or_else(|| {
            task.error
                .as_deref()
                .map(DownloadFailure::from_stored_reason)
                .unwrap_or(DownloadFailure::DownloadFailed)
        });
        json!({
            "code":code,
            "message":code.message(),
            "http_status":http.and_then(|details| details.http_status),
            "stage":http.map(|details| details.stage)
        })
    });
    json!({
        "id":task.id,"name":name,"engine":engine,"state":task.state.as_str(),
        "output":task.output_path,"progress":task.progress,
        "error":error,
        "proxy":{
            "requested":task.policy.proxy_mode,
            "effective":http.and_then(|details| details.effective_proxy)
        }
    })
}

fn object(fields: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":fields,"required":required,"additionalProperties":false})
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":object(properties, required)})
}

fn item_fields(characters: bool) -> Value {
    let mut fields = json!({
        "show_progress":{"type":"boolean","description":"Override the batch window choice for this item. Defaults to quiet."},
        "source":{"type":"string","description":SOURCE_SCHEMA},
        "destination":{"type":"string","description":"Absolute download directory. Omit to use OnesDM's default."},
        "plugin":{"type":"string","description":"Installed engine ID, or builtin to select the native engine. Omit for automatic selection."},
        "options":{"type":"object","description":"Options declared by the selected plugin"},
        "download_threads":thread_limit(),
        "proxy_mode":{"type":"string","enum":["inherit","direct","system_route","tun_direct"],"description":"Built-in HTTP(S) only. inherit uses desktop settings. system_route (or legacy direct) skips HTTP proxies and follows OS routing including TUN. tun_direct requires the configured network interface or local DIRECT listener."},
        "user_agent":{"type":"string","maxLength":1024,"description":"Custom User-Agent for an HTTP(S) download or a media plugin that accepts HTTP headers."},
        "user_agent_profile":{"type":"string","enum":["desktop_chrome","iphone_safari","android_chrome"],"description":"Device User-Agent for the requests. Use either a profile or user_agent."},
        "referer":{"type":"string","description":"HTTP(S) page URL used as Referer for this download."},
        "accept_language":{"type":"string","maxLength":256,"description":"HTTP Accept-Language, for example zh-CN,zh;q=0.9."},
        "media_type":{"type":"string","enum":["hls","dash"],"description":"Force the built-in stream engine for an already identified playlist, including a URL without a file extension."}
    });
    if characters {
        fields["character"] = json!({"type":"string","description":"Apply this saved download character. Explicit item fields win."});
    }
    fields
}

fn character_fields() -> Value {
    let mut defaults = item_fields(false);
    let map = defaults.as_object_mut().unwrap();
    map.remove("source");
    map.remove("referer");
    for value in map.values_mut() {
        if let Some(kind) = value["type"].as_str() {
            value["type"] = json!([kind, "null"]);
        }
        if let Some(values) = value["enum"].as_array_mut() {
            values.push(Value::Null);
        }
    }
    json!({
        "id":{"type":"string","pattern":"^[a-z0-9][a-z0-9-]{0,39}$"},
        "name":{"type":"string","description":"Display name. Required for new characters."},
        "description":{"type":"string","description":"When to use this role. Required for new characters."},
        "instructions":{"type":"string","description":"Optional user-authored download guidance. Keep credentials and expiring URLs outside the Skill."},
        "defaults":object(defaults, &[]),
        "replace":{"type":"boolean","default":false},
    })
}

fn probe_fields() -> Value {
    let mut fields = item_fields(false);
    fields.as_object_mut().unwrap().retain(|field, _| {
        matches!(
            field.as_str(),
            "source" | "user_agent" | "user_agent_profile" | "referer" | "accept_language"
        )
    });
    fields
}

fn task_limit() -> Value {
    json!({"type":"integer","minimum":1,"maximum":32})
}
fn color_fields(keys: &[&str], description: &str) -> Value {
    let fields: Map<String, Value> = keys
        .iter()
        .map(|&key| {
            (
                key.into(),
                json!({"type":["string","null"],"pattern":"^#[a-fA-F0-9]{6}$"}),
            )
        })
        .collect();
    let mut schema = object(json!(fields), &[]);
    schema["description"] = json!(description);
    schema
}
fn thread_limit() -> Value {
    json!({"type":"integer","minimum":1,"maximum":256})
}
