use std::{
    fs,
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use ndm_core::{Store, plugins::PluginRegistry};
use ndm_host::harness::{CodexCli, Harness, Target};
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
    let path = profile.join("fixture.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path)?);
    let options = zip::write::SimpleFileOptions::default();
    let engine = format!("ndm-fixture{}", std::env::consts::EXE_SUFFIX);
    zip.start_file("plugin.json", options)?;
    serde_json::to_writer(
        &mut zip,
        &json!({
            "schema":1,"id":"fixture","version":"0.1.0","name":{"en":"Fixture"},
            "description":{"en":"Test engine"},"engine":engine,"engine_protocol":1,
            "inputs":[{"schemes":["fixture"]}],"limits":{"active_tasks":1},
            "tools":[{"id":"download","name":{"en":"Fixture download"},"description":{"en":"Fixture"} }]
        }),
    )?;
    zip.start_file(&engine, options.unix_permissions(0o755))?;
    zip.write_all(b"never executed")?;
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
#[ignore = "requires Codex CLI on PATH and uses an isolated configuration"]
fn installs_and_removes_the_codex_integration() {
    if let Some(directory) = std::env::var_os("NDM_TEST_CODEX_DIRECTORY") {
        let directory = PathBuf::from(directory);
        let profile = directory.join("profile");
        fs::create_dir_all(&profile).unwrap();
        let target = Target {
            executable: env!("CARGO_BIN_EXE_ndm-host").into(),
            profile,
            home: directory,
        };
        CodexCli.install(&target).unwrap();
        let status = CodexCli.status(&target).unwrap();
        assert!(status.registered && status.skill_installed);
        CodexCli.remove(&target).unwrap();
        let status = CodexCli.status(&target).unwrap();
        assert!(!status.registered && !status.skill_installed);
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let codex_home = directory.path().join("codex");
    fs::create_dir(&codex_home).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "installs_and_removes_the_codex_integration",
            "--ignored",
        ])
        .env("NDM_TEST_CODEX_DIRECTORY", directory.path())
        .env("CODEX_HOME", codex_home)
        .status()
        .unwrap();
    assert!(status.success());
}
