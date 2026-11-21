//! Local stdio transport used by the optional Codex CLI integration.
use std::{
    io::{self, BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use ndm_core::{
    BatchLimit, NewTask, Store, Task, TaskCommand, TaskId, TaskOptions, TaskPolicy, TaskState,
    plugins::{InstalledPlugin, PluginRegistry, built_in_source, parse_source},
};
use ndm_plugin_sdk::{read_message, write_message};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const PROTOCOL: &str = "2025-11-25";
const SOURCE_SCHEMA: &str = "An HTTP(S), FTP, FTPS, SFTP, or installed plugin source. For local files, pass an absolute path.";

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
    loop {
        match receive.recv_timeout(Duration::from_secs(1)) {
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
            Err(mpsc::RecvTimeoutError::Timeout) if subscribed => {
                let next = json!(server.tools());
                if tools != next {
                    tools = next;
                    write_message(
                        &mut output,
                        &json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
                    )?;
                }
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
                "List recent tasks and their state, including completed output paths.",
                json!({"limit":{"type":"integer","minimum":1,"maximum":128}}),
                &[],
            ),
            tool(
                "ndm_task_status",
                "Look up an NDM Next task by ID.",
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
        for plugin in self.registry.list().unwrap_or_default() {
            for action in &plugin.manifest.tools {
                let mut fields = item_fields();
                fields
                    .as_object_mut()
                    .expect("tool fields are an object")
                    .remove("plugin");
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
                    &plugin_tool_name(&plugin, &action.id),
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
            "ndm_list_tasks" => {
                let args: List = parse_arguments(args)?;
                let tasks = self.store.list().map_err(|_| "Could not read tasks.")?;
                let tasks: Vec<_> = tasks
                    .iter()
                    .rev()
                    .take(args.limit.unwrap_or(32).clamp(1, 128))
                    .map(task_json)
                    .collect();
                Ok(json!({"tasks":tasks}))
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
                {
                    for action in &plugin.manifest.tools {
                        if plugin_tool_name(&plugin, &action.id) != name {
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
                .task_options(&source, item.plugin.as_deref(), item.options)
                .map_err(|_| "Plugin or plugin options do not match the source.")?;
            if options.is_none() && !built_in_source(&source) {
                return Err("No installed download engine handles this source.");
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
                headers: Default::default(),
            };
            tasks.push((
                TaskId::new_v4(),
                request,
                options.unwrap_or(TaskOptions::Auto),
                TaskPolicy {
                    batch: batch_limit,
                    download_threads: threads,
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskArgument {
    task_id: TaskId,
}

fn task_json(task: &Task) -> Value {
    let state = match task.state {
        TaskState::Prepared => "prepared",
        TaskState::Queued => "queued",
        TaskState::Running => "running",
        TaskState::Paused => "paused",
        TaskState::Completed => "completed",
        TaskState::Failed => "failed",
    };
    json!({"id":task.id,"state":state,"output":task.output_path,"progress":task.progress})
}

fn plugin_tool_name(plugin: &InstalledPlugin, action: &str) -> String {
    format!(
        "ndm_{}__{}",
        plugin.manifest.id.replace('-', "_"),
        action.replace('-', "_")
    )
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
        "download_threads":thread_limit()
    })
}

fn task_limit() -> Value {
    json!({"type":"integer","minimum":1,"maximum":32})
}
fn thread_limit() -> Value {
    json!({"type":"integer","minimum":1,"maximum":256})
}
