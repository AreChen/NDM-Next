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
            "manual_inputs":[{"schemes":["https"],"hosts":["fixture.test"],"suffixes":[".demo"],"exclude_suffixes":[".ignore.demo"]}],
            "options":[{"key":"label","label":{"en":"Output label"},"kind":"text","max_length":80}],
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
    let routes = server.call(
        "tools/call",
        json!({"name":"ndm_network_routes","arguments":{}}),
    );
    assert!(routes["structuredContent"]["tun_direct_exit"].is_null());
    let rejected = server.call(
        "tools/call",
        json!({"name":"ndm_download","arguments":{
        "items":[{"source":"https://example.org/model.bin", "proxy_mode":"tun_direct"}]}}),
    );
    assert_eq!(rejected["isError"], true);
    assert!(store.list().unwrap().is_empty());
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
    let create = |path: &str, name: Option<&str>| {
        let id = TaskId::new_v4();
        store
            .prepare(
                id,
                &NewTask {
                    url: format!("https://example.test/{path}"),
                    destination: profile.path().into(),
                    suggested_filename: name.map(str::to_owned),
                    source_url: None,
                    expected_size: None,
                    headers: Default::default(),
                },
            )
            .unwrap();
        id
    };
    let first = create("repo?FilePath=data%2Ffirst.zip", None);
    store.commit(first).unwrap();
    let second = create("second.zip", Some("second.zip"));
    store.commit(second).unwrap();
    create("prepared.zip", Some("prepared.zip"));
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
    assert_eq!(next["structuredContent"]["tasks"][0]["name"], "first.zip");
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
        for command in *b"RWRWW" {
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
    assert_eq!(
        store.get(id).unwrap().unwrap().policy.show_progress,
        Some(false)
    );
    assert!(store.progress_window_requests().unwrap().is_empty());
    for visible in [true, false] {
        let reply = mcp.call(
            "tools/call",
            json!({"name":"ndm_progress_window","arguments":{"task_id":id,"visible":visible}}),
        );
        assert_eq!(reply["structuredContent"]["requested"], true);
        assert_eq!(
            store.progress_window_requests().unwrap(),
            vec![(id, visible)]
        );
        assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Queued);
    }
    store.finish_progress_window_request(id, true).unwrap();
    assert_eq!(store.progress_window_requests().unwrap(), vec![(id, false)]);
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn batch_and_plugin_window_options_and_tun_routes_are_persisted() {
    use std::{io::Read, os::unix::net::UnixListener};
    let profile = tempfile::tempdir().unwrap();
    let store = Store::open(&profile.path().join("tasks.db")).unwrap();
    store.set_ai_enabled(true).unwrap();
    PluginRegistry::new(profile.path())
        .import(&fixture(profile.path()).unwrap())
        .unwrap();
    fs::write(
        profile.path().join("preferences.json"),
        br#"{"tun_egress":{"local_port":7893}}"#,
    )
    .unwrap();
    let socket =
        UnixListener::bind(ndm_core::unix_instance::socket_path(profile.path()).unwrap()).unwrap();
    let desktop = std::thread::spawn(move || {
        for command in *b"RWRW" {
            let (mut stream, _) = socket.accept().unwrap();
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            assert_eq!(byte, [command]);
            stream.write_all(&[1]).unwrap();
        }
    });
    let mut mcp = Server::start(profile.path());
    mcp.call("initialize", json!({"protocolVersion":"2025-11-25"}));
    let routes = mcp.call("tools/call", json!({"name":"ndm_network_routes"}));
    assert_eq!(
        routes["structuredContent"]["tun_direct_exit"],
        "local_direct_listener"
    );
    let batch=mcp.call("tools/call",json!({"name":"ndm_download","arguments":{
        "show_progress":true,
        "items":[{"source":"https://example.org/a","proxy_mode":"tun_direct"},
                 {"source":"https://example.org/b","proxy_mode":"system_route","show_progress":false}]
    }}));
    let plugin = mcp.call(
        "tools/call",
        json!({"name":"ndm_fixture__download","arguments":{"source":"fixture://example/b"}}),
    );
    let ids: Vec<TaskId> =
        serde_json::from_value(batch["structuredContent"]["task_ids"].clone()).unwrap();
    let plugin_id =
        serde_json::from_value(plugin["structuredContent"]["task_ids"][0].clone()).unwrap();
    assert_eq!(
        store.get(ids[0]).unwrap().unwrap().policy.proxy_mode,
        ndm_core::ProxyMode::TunDirect
    );
    assert_eq!(
        store.get(ids[1]).unwrap().unwrap().policy.proxy_mode,
        ndm_core::ProxyMode::SystemRoute
    );
    assert_eq!(
        store.get(plugin_id).unwrap().unwrap().policy.show_progress,
        Some(false)
    );
    assert_eq!(
        store.progress_window_requests().unwrap(),
        vec![(ids[0], true)]
    );
    desktop.join().unwrap();
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

    let engine = registry.import(&fixture(&target.profile).unwrap()).unwrap();
    registry
        .save_option_defaults(
            &engine,
            serde_json::from_value(json!({"label":"private-user-setting"})).unwrap(),
        )
        .unwrap();
    Harness::ClaudeCode.install(&target).unwrap();
    Harness::OpenCode.install(&target).unwrap();
    Harness::DeepSeek.install(&target).unwrap();
    sync_skills(&target).unwrap();
    let guides = root.join("references/plugins");
    let guide = guides.join("plugin-fixture.md");
    assert!(
        fs::read_to_string(root.join("SKILL.md"))
            .unwrap()
            .contains("ndm_list_engines")
    );
    let generated = fs::read_to_string(&guide).unwrap();
    for term in [
        "ndm_fixture__download",
        "Automatic sources",
        "Explicit selection",
        "fixture.test",
        ".ignore.demo",
        "`label`",
        "80 characters",
    ] {
        assert!(generated.contains(term), "missing {term}");
    }
    assert!(!generated.contains("private-user-setting"));
    let sdk = root.join("references/sdk");
    let example = sdk.join("examples/file-copy.json");
    assert!(sdk.join("quickstart.md").is_file() && example.is_file());
    fs::remove_file(&example).unwrap();
    assert!(!Harness::Codex.status(&target).unwrap().skill_installed);
    fs::write(
        sdk.join("README.md"),
        "<!-- Managed by the NDM Next desktop integration. -->\nOlder SDK\n",
    )
    .unwrap();
    sync_skills(&target).unwrap();
    assert!(Harness::Codex.status(&target).unwrap().skill_installed);
    assert!(
        fs::read_to_string(sdk.join("README.md"))
            .unwrap()
            .contains("runtime_dependencies")
    );
    assert!(example.is_file());

    let update = plugin_package(&target.profile, "0.2.0", Some("Engine usage example")).unwrap();
    registry.import(&update).unwrap();
    sync_skills(&target).unwrap();
    let text = fs::read_to_string(&guide).unwrap();
    assert!(text.contains("0.2.0") && text.contains("Engine usage example"));
    for other in [".claude/skills", ".config/opencode/skills", ".dsh/skills"] {
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
    assert!(sdk.join("quickstart.md").is_file());
    assert!(guides.join("plugin-local.md").exists());
    registry.set_ai_access("fixture", true).unwrap();
    sync_skills(&target).unwrap();
    assert!(guide.exists());
    store.set_ai_enabled(false).unwrap();
    sync_skills(&target).unwrap();
    assert!(!guide.exists());
    assert!(sdk.join("README.md").is_file() && example.is_file());
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
