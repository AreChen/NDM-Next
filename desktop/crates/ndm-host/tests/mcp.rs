use std::{
    fs,
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use ndm_core::{NewTask, Store, TaskId, plugins::PluginRegistry};
use ndm_host::harness::{Harness, Target, sync_skills};
use ndm_plugin_sdk::{read_message, write_message};
use serde_json::{Value, json};

struct Server {
    child: Child,
    input: ChildStdin,
    messages: mpsc::Receiver<Value>,
}

impl Server {
    fn start(profile: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ndm-host"))
            .arg("--mcp")
            .arg("--data-dir")
            .arg(profile)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, messages) = mpsc::channel();
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            while let Ok(Some(message)) = read_message::<Value>(&mut output) {
                if send.send(message).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            messages,
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        write_message(
            &mut self.input,
            &json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
        )
        .unwrap();
        loop {
            let reply = self.next();
            if reply["id"] == 1 {
                assert!(reply.get("error").is_none(), "MCP request failed");
                return reply["result"].clone();
            }
        }
    }

    fn next(&self) -> Value {
        self.messages.recv_timeout(Duration::from_secs(5)).unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fixture(profile: &Path) -> io::Result<PathBuf> {
    plugin_package(profile, "0.1.0", None)
}

fn plugin_package(profile: &Path, version: &str, guide: Option<&str>) -> io::Result<PathBuf> {
    let path = profile.join("fixture.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path)?);
    let options = zip::write::SimpleFileOptions::default();
    let engine = format!("ndm-fixture{}", std::env::consts::EXE_SUFFIX);
    zip.start_file("plugin.json", options)?;
    serde_json::to_writer(
        &mut zip,
        &json!({
            "schema":1,"id":"fixture","version":version,"name":{"en":"Fixture"},
            "description":{"en":"Test engine"},"engine":engine,"engine_protocol":1,
            "inputs":[{"schemes":["fixture"]}],"limits":{"active_tasks":1},
            "skill": guide.map(|_| "docs/guide.md"),
            "tools":[{"id":"download","name":{"en":"Fixture download"},"description":{"en":"Fixture"} }]
        }),
    )?;
    zip.start_file(&engine, options.unix_permissions(0o755))?;
    zip.write_all(b"never executed")?;
    if let Some(guide) = guide {
        zip.start_file("docs/guide.md", options)?;
        zip.write_all(guide.as_bytes())?;
    }
    zip.finish()?;
    Ok(path)
}

#[test]
fn tools_follow_opt_in_and_current_plugin_registry() {
    let profile = tempfile::tempdir().unwrap();
    let store = Store::open(&profile.path().join("tasks.db")).unwrap();
    let mut server = Server::start(profile.path());
    let info = server.call("initialize", json!({"protocolVersion":"2025-11-25"}));
    assert_eq!(info["capabilities"]["tools"]["listChanged"], true);
    write_message(
        &mut server.input,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .unwrap();
    assert_eq!(server.call("tools/list", json!({})), json!({"tools":[]}));
    store.set_ai_enabled(true).unwrap();
    let tools = server.call("tools/list", json!({}));
    assert!(
        tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "ndm_download")
    );
    let package = fixture(profile.path()).unwrap();
    PluginRegistry::new(profile.path())
        .import(&package)
        .unwrap();
    let change = server.next();
    assert_eq!(change["method"], "notifications/tools/list_changed");
    let tools = server.call("tools/list", json!({}));
    assert!(
        tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "ndm_fixture__download")
    );
    let discovery =
        || json!({"name":"ndm_list_engines","arguments":{"source":"fixture://example/download"}});
    let engines = server.call("tools/call", discovery());
    assert_eq!(engines["structuredContent"]["automatic_engine"], "fixture");
    assert_eq!(
        engines["structuredContent"]["plugins"][0]["tools"],
        json!(["ndm_fixture__download"])
    );
    PluginRegistry::new(profile.path())
        .set_ai_access("fixture", false)
        .unwrap();
    assert_eq!(server.next()["method"], "notifications/tools/list_changed");
    assert_eq!(
        server.call("tools/call", discovery())["structuredContent"]["plugins"],
        json!([])
    );
    for (name, arguments) in [
        ("ndm_fixture__download", json!({"source":"fixture://test"})),
        (
            "ndm_download",
            json!({"items":[{"source":"fixture://test"}]}),
        ),
        (
            "ndm_download",
            json!({"items":[{"source":"fixture://test","plugin":"fixture"}]}),
        ),
    ] {
        assert_eq!(
            server.call("tools/call", json!({"name":name,"arguments":arguments}))["isError"],
            true
        );
    }
    let result = server.call(
        "tools/call",
        json!({
            "name":"ndm_set_download_limits","arguments":{"active_tasks":5,"download_threads":8}
        }),
    );
    assert_eq!(
        result["structuredContent"],
        json!({"active_tasks":5,"download_threads":8})
    );
    assert_eq!(store.download_limits().unwrap().unwrap().active_tasks, 5);
    store.set_ai_enabled(false).unwrap();
    assert_eq!(server.call("tools/list", json!({}))["tools"], json!([]));
    assert_eq!(
        server.call("tools/call", json!({"name":"ndm_get_download_limits"}))["isError"],
        true
    );
}

#[test]
fn task_pages_filter_state_and_continue_without_repeating_ids() {
    let profile = tempfile::tempdir().unwrap();
    let store = Store::open(&profile.path().join("tasks.db")).unwrap();
    store.set_ai_enabled(true).unwrap();
    let create = |name: &str| {
        let id = TaskId::new_v4();
        store
            .prepare(
                id,
                &NewTask {
                    url: format!("https://example.test/{name}"),
                    destination: profile.path().into(),
                    suggested_filename: Some(name.into()),
                    source_url: None,
                    expected_size: None,
                    headers: Default::default(),
                },
            )
            .unwrap();
        id
    };
    let first = create("first.zip");
    store.commit(first).unwrap();
    let second = create("second.zip");
    store.commit(second).unwrap();
    create("prepared.zip");
    let mut server = Server::start(profile.path());
    server.call("initialize", json!({"protocolVersion":"2025-11-25"}));
    let page = server.call(
        "tools/call",
        json!({
            "name":"ndm_list_tasks","arguments":{"state":"queued","limit":1}
        }),
    );
    let page = &page["structuredContent"];
    assert_eq!(page["tasks"][0]["id"], json!(second));
    assert_eq!(page["tasks"][0]["name"], "second.zip");
    let next = server.call("tools/call", json!({
        "name":"ndm_list_tasks","arguments":{"state":"queued","limit":1,"cursor":page["next_cursor"]}
    }));
    assert_eq!(next["structuredContent"]["tasks"][0]["id"], json!(first));
    assert!(next["structuredContent"]["next_cursor"].is_null());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn download_overrides_and_failure_diagnostics_round_trip_through_mcp() {
    use ndm_core::{HttpOptions, ProxyMode, ProxyOptions, Scheduler, SchedulerOptions, TaskState};
    use std::{
        io::{BufRead, Read},
        net::TcpListener,
        os::unix::net::UnixListener,
        sync::Arc,
    };

    let profile = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&profile.path().join("tasks.db")).unwrap());
    store.set_ai_enabled(true).unwrap();
    // Register the local readiness/wake fixture without launching the UI.
    let socket =
        UnixListener::bind(ndm_core::unix_instance::socket_path(profile.path()).unwrap()).unwrap();
    let desktop = std::thread::spawn(move || {
        for command in *b"RW" {
            let (mut connection, _) = socket.accept().unwrap();
            let mut received = [0];
            connection.read_exact(&mut received).unwrap();
            assert_eq!(received, [command]);
            connection.write_all(&[1]).unwrap();
        }
    });
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/repo?token=private-link",
        origin.local_addr().unwrap()
    );
    let http = std::thread::spawn(move || {
        for (path, has_range, response) in [
            (
                "/repo",
                true,
                "302 Found\r\nLocation: /model?token=private-redirect\r\nContent-Length: 0\r\n",
            ),
            ("/model", true, "403 Forbidden\r\nContent-Length: 0\r\n"),
            (
                "/repo",
                false,
                "302 Found\r\nLocation: /model?token=private-redirect\r\nContent-Length: 0\r\n",
            ),
            (
                "/model",
                false,
                "401 Unauthorized\r\nContent-Length: 12\r\n",
            ),
        ] {
            let (mut connection, _) = origin.accept().unwrap();
            connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&connection);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with(&format!("GET {path}?")));
            let mut headers = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                headers.push_str(&line.to_ascii_lowercase());
            }
            assert!(headers.contains("user-agent: private-agent\r\n"));
            assert_eq!(headers.contains("range: bytes=0-0\r\n"), has_range);
            let body = if response.starts_with("401") {
                "private-body"
            } else {
                ""
            };
            let reply = format!("HTTP/1.1 {response}Connection: close\r\n\r\n{body}");
            connection.write_all(reply.as_bytes()).unwrap();
        }
    });

    let mut mcp = Server::start(profile.path());
    mcp.call("initialize", json!({"protocolVersion":"2025-11-25"}));
    let reply = mcp.call(
        "tools/call",
        json!({
            "name":"ndm_download",
            "arguments":{"destination":profile.path(),"items":[{
                "source":url,"proxy_mode":"direct","user_agent":"private-agent"
            }]}
        }),
    );
    assert_ne!(reply["isError"], true);
    let id: TaskId =
        serde_json::from_value(reply["structuredContent"]["task_ids"][0].clone()).unwrap();
    assert_eq!(
        store.get(id).unwrap().unwrap().policy.proxy_mode,
        ProxyMode::Direct
    );
    desktop.join().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let scheduler = Scheduler::start(
            store.clone(),
            SchedulerOptions {
                http: HttpOptions {
                    proxy: Some(ProxyOptions {
                        url: "http://127.0.0.1:0".into(),
                        username: Some("private-proxy".into()),
                        password: Some("private-password".into()),
                    }),
                    user_agent: Some("ndm-default".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if store.get(id).unwrap().unwrap().state == TaskState::Failed {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        scheduler.shutdown().await;
    });
    http.join().unwrap();
    let result = mcp.call(
        "tools/call",
        json!({
            "name":"ndm_task_status","arguments":{"task_id":id}
        }),
    );
    let task = &result["structuredContent"];
    assert_eq!(task["state"], "failed");
    assert_eq!(
        task["proxy"],
        json!({"requested":"direct","effective":"direct"})
    );
    assert_eq!(task["error"]["code"], "http_status");
    assert_eq!(task["error"]["http_status"], 401);
    assert_eq!(task["error"]["stage"], "full_download");
    assert_eq!(task["progress"]["http"]["range_probe_status"], 403);
    assert_eq!(task["progress"]["http"]["redirect_count"], 1);
    assert_eq!(task["progress"]["downloaded"], 0);
    assert!(!result.to_string().contains("private-"));
}

#[test]
fn managed_skill_refreshes_legacy_installs_and_plugin_guides() {
    let root = tempfile::tempdir().unwrap();
    let target = Target::new(
        env!("CARGO_BIN_EXE_ndm-host").into(),
        root.path().join("profile"),
        root.path().join("home"),
    );
    fs::create_dir(&target.profile).unwrap();
    let store = Store::open(&target.profile.join("tasks.db")).unwrap();
    store.set_ai_enabled(true).unwrap();
    let root = target.home.join(".agents/skills/ndm-next");
    let registry = PluginRegistry::new(&target.profile);
    sync_skills(&target).unwrap();
    assert!(!root.exists());
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("SKILL.md"),
        "<!-- Managed by the NDM Next desktop integration. -->\nOld guide\n",
    )
    .unwrap();

    registry.import(&fixture(&target.profile).unwrap()).unwrap();
    Harness::ClaudeCode.install(&target).unwrap();
    Harness::OpenCode.install(&target).unwrap();
    sync_skills(&target).unwrap();
    let guides = root.join("references/plugins");
    let guide = guides.join("plugin-fixture.md");
    assert!(
        fs::read_to_string(root.join("SKILL.md"))
            .unwrap()
            .contains("ndm_list_engines")
    );
    assert!(
        fs::read_to_string(&guide)
            .unwrap()
            .contains("ndm_fixture__download")
    );

    let update = plugin_package(&target.profile, "0.2.0", Some("Engine usage example")).unwrap();
    registry.import(&update).unwrap();
    sync_skills(&target).unwrap();
    let text = fs::read_to_string(&guide).unwrap();
    assert!(text.contains("0.2.0") && text.contains("Engine usage example"));
    for other in [".claude/skills", ".config/opencode/skills"] {
        assert_eq!(
            fs::read_to_string(
                target
                    .home
                    .join(other)
                    .join("ndm-next/references/plugins/plugin-fixture.md")
            )
            .unwrap(),
            text,
        );
    }
    assert!(!target.home.join(".pi").exists());
    fs::write(guides.join("plugin-local.md"), "User guide").unwrap();
    registry.set_ai_access("fixture", false).unwrap();
    sync_skills(&target).unwrap();
    assert!(!guide.exists());
    assert!(guides.join("plugin-local.md").exists());
    registry.set_ai_access("fixture", true).unwrap();
    sync_skills(&target).unwrap();
    assert!(guide.exists());
    store.set_ai_enabled(false).unwrap();
    sync_skills(&target).unwrap();
    assert!(!guide.exists());
    assert!(
        fs::read_to_string(guides.join("index.md"))
            .unwrap()
            .contains("AI access is off")
    );
}

#[test]
#[ignore = "requires Codex CLI on PATH and uses an isolated configuration"]
fn codex_cli_reads_the_shared_registration() {
    let directory = tempfile::tempdir().unwrap();
    let target = Target::new(
        env!("CARGO_BIN_EXE_ndm-host").into(),
        directory.path().join("profile"),
        directory.path().to_path_buf(),
    );
    let list = || {
        let output = Command::new("codex")
            .current_dir(directory.path())
            .env("CODEX_HOME", target.home.join(".codex"))
            .args(["mcp", "list", "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Codex could not read the isolated configuration"
        );
        serde_json::from_slice::<Vec<Value>>(&output.stdout).unwrap()
    };
    Harness::Codex.install(&target).unwrap();
    let servers = list();
    let server = servers
        .iter()
        .find(|server| server["name"] == "ndm-next")
        .unwrap();
    assert_eq!(server["transport"]["command"], json!(target.executable));
    assert_eq!(
        server["transport"]["args"],
        json!(["--mcp", "--data-dir", target.profile.to_str().unwrap()])
    );
    Harness::Codex.remove(&target).unwrap();
    assert!(!list().iter().any(|server| server["name"] == "ndm-next"));
}
