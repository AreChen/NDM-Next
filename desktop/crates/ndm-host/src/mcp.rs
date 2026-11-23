//! Local stdio transport used by the optional Codex CLI integration.
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
    validate_user_agent,
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
                            "serverInfo":{"name":"ndm-next","version":env!("CARGO_PKG_VERSION")}
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
        let task_id = json!({"task_id":{"type":"string","format":"uuid"}});
        let mut tools = vec![
            tool(
                "ndm_download",
                "Queue downloads in NDM Next. Omit destination to use the configured download directory. Batch settings apply only to these items.",
                json!({
                    "items":{"type":"array","minItems":1,"maxItems":128,"items":object(item_fields(), &["source"])},
                    "destination":{"type":"string","description":"Absolute directory"},
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
                "ndm_task_status",
                "Look up a task, including recent throughput, idle time and redacted HTTP diagnostics when available.",
                task_id.clone(),
                &["task_id"],
            ),
            tool(
                "ndm_pause_task",
                "Request that NDM Next pause a task and save resumable data.",
                task_id.clone(),
                &["task_id"],
            ),
            tool(
                "ndm_resume_task",
                "Resume or retry an NDM Next task.",
                task_id,
                &["task_id"],
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
        for plugin in self
            .registry
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.ai_enabled)
        {
            for action in &plugin.manifest.tools {
                let mut fields = item_fields();
                let fields_object = fields.as_object_mut().expect("tool fields are an object");
                for key in ["plugin", "proxy_mode", "user_agent"] {
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
            return Err("Enable AI integration in NDM Next first.");
        }
        let name = params["name"].as_str().ok_or("Missing tool name.")?;
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        match name {
            "ndm_download" => self.download(parse_arguments(args)?),
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
        for item in batch.items {
            let source = parse_source(&item.source).map_err(|_| "Invalid download source.")?;
            let options = self
                .registry
                .ai_task_options(&source, item.plugin.as_deref(), item.options)
                .map_err(|_| "Plugin or plugin options do not match the source.")?;
            if options.is_none() && !built_in_source(&source) {
                return Err("No installed download engine handles this source.");
            }
            let proxy_mode = item.proxy_mode.unwrap_or_default();
            if (proxy_mode == ProxyMode::Direct || item.user_agent.is_some())
                && (options.is_some() || !matches!(source.scheme(), "http" | "https"))
            {
                return Err("Proxy and User-Agent overrides apply to built-in HTTP(S) downloads.");
            }
            let mut headers = std::collections::BTreeMap::new();
            if let Some(user_agent) = item.user_agent {
                validate_user_agent(&user_agent).map_err(|_| "Invalid User-Agent.")?;
                headers.insert("user-agent".into(), user_agent);
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
                .arg("--handoff")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| "Could not launch NDM Next.")?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        if ndm_core::wait_for_desktop_ready(&self.profile, Duration::from_secs(20)).unwrap_or(false)
        {
            Ok(())
        } else {
            Err("NDM Next did not become ready.")
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
    items: Vec<Item>,
    destination: Option<PathBuf>,
    batch_active_tasks: Option<u32>,
    download_threads: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    source: String,
    destination: Option<PathBuf>,
    plugin: Option<String>,
    download_threads: Option<u32>,
    proxy_mode: Option<ProxyMode>,
    user_agent: Option<String>,
    #[serde(default)]
    options: Map<String, Value>,
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
        .or_else(|| task.request.suggested_filename.as_deref().map(Into::into));
    let http = task.progress.as_ref().and_then(|progress| progress.http);
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

fn item_fields() -> Value {
    json!({
        "source":{"type":"string","description":SOURCE_SCHEMA},
        "destination":{"type":"string","description":"Absolute download directory. Omit to use NDM Next's default."},
        "plugin":{"type":"string","description":"Optional installed plugin ID"},
        "options":{"type":"object","description":"Options declared by the selected plugin"},
        "download_threads":thread_limit(),
        "proxy_mode":{"type":"string","enum":["inherit","direct"],"description":"For built-in HTTP(S). Direct bypasses both NDM Next and system proxies for this item."},
        "user_agent":{"type":"string","maxLength":1024,"description":"Optional User-Agent for this built-in HTTP(S) item. Omit to use app defaults."}
    })
}

fn task_limit() -> Value {
    json!({"type":"integer","minimum":1,"maximum":32})
}
fn thread_limit() -> Value {
    json!({"type":"integer","minimum":1,"maximum":256})
}
