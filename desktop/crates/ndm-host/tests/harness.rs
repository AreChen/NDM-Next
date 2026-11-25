use std::{fs, io, path::Path};

use ndm_host::harness::{Harness, McpStatus, Status, Target};
use serde_json::{Value, json};
use toml_edit::{Array, DocumentMut, value};

fn target(root: &Path) -> Target {
    // Match the bundle layout and paths with spaces that a Finder launch uses.
    // A plain fixture file also proves registration never starts the host or CLI.
    let bundle = root.join("My OnesDM.app/Contents/MacOS");
    fs::create_dir_all(&bundle).unwrap();
    let executable = bundle.join(if cfg!(windows) {
        "ndm-host.exe"
    } else {
        "ndm-host"
    });
    fs::write(&executable, b"fixture").unwrap();
    Target::new(executable, root.join("下载记录"), root.join("home"))
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn check_skill_bundle(root: &Path) {
    for guide in [
        "SKILL.md",
        "references/downloads.md",
        "references/diagnostics.md",
        "references/plugins/index.md",
        "references/sdk/quickstart.md",
        "references/sdk/README.md",
    ] {
        let path = root.join(guide);
        let text = fs::read_to_string(&path).unwrap();
        for link in text.split("](").skip(1) {
            let (link, _) = link.split_once(')').expect("closed Markdown link");
            let relative = link.split('#').next().unwrap();
            if !relative.is_empty() && !relative.starts_with("https://") {
                assert!(
                    path.parent().unwrap().join(relative).is_file(),
                    "{guide} links to missing {link}"
                );
            }
        }
    }
    let sdk = root.join("references/sdk");
    assert!(
        fs::read_to_string(sdk.join("README.md"))
            .unwrap()
            .ends_with(include_str!("../../ndm-plugin-sdk/README.md"))
    );
    assert!(
        fs::read_to_string(sdk.join("examples/file-copy.rs"))
            .unwrap()
            .ends_with(include_str!("../../ndm-plugin-sdk/examples/file-copy.rs"))
    );
    let bytes = fs::read(sdk.join("examples/file-copy.json")).unwrap();
    let manifest: ndm_plugin_sdk::PluginManifest = serde_json::from_slice(&bytes).unwrap();
    manifest.validate().unwrap();
    let mut example: Value = serde_json::from_slice(&bytes).unwrap();
    example.as_object_mut().unwrap().remove("$comment");
    assert_eq!(
        example,
        serde_json::from_str::<Value>(include_str!("../../ndm-plugin-sdk/examples/file-copy.json"))
            .unwrap()
    );
}

#[test]
fn all_clients_register_and_remove_their_own_mcp_and_skill_without_a_cli() {
    let root = tempfile::tempdir().unwrap();
    let target = target(root.path());
    let codex_file = target.home.join(".codex/config.toml");
    let claude_file = target.home.join(".claude.json");
    let pi_file = target.home.join(".pi/agent/mcp.json");
    let opencode_file = target.home.join(".config/opencode/opencode.jsonc");
    let skill_roots = [
        ".agents/skills",
        ".claude/skills",
        ".pi/agent/skills",
        ".config/opencode/skills",
        ".dsh/skills",
    ];
    for client in Harness::ALL {
        assert_eq!(client.status(&target).unwrap(), Status::default());
    }
    assert!(!target.home.exists());
    write(
        &codex_file,
        "# Keep this comment\nmodel = \"other-model\"\n",
    );
    write(
        &claude_file,
        "{\"projects\": {\"workspace\": {\"existing\": true}}}\n",
    );
    write(
        &pi_file,
        "{\"mcpServers\": {\"another\": {\"command\": \"existing\"}}}\n",
    );
    write(
        &opencode_file,
        "{\n  // Keep OpenCode preferences\n  \"model\": \"provider/model\",\n}\n",
    );
    for (client, skill_root) in Harness::ALL.into_iter().zip(skill_roots) {
        client.install(&target).unwrap();
        client.install(&target).unwrap();
        assert!(client.status(&target).unwrap().installed());
        let root = target.home.join(skill_root).join("onesdm");
        check_skill_bundle(&root);
        assert!(
            fs::read_to_string(root.join("SKILL.md"))
                .unwrap()
                .contains("ndm_task_status")
        );
    }
    let codex = fs::read_to_string(&codex_file).unwrap();
    assert!(codex.contains("# Keep this comment"));
    let parsed = codex.parse::<DocumentMut>().unwrap();
    assert_eq!(
        parsed["mcp_servers"]["onesdm-mcp"]["command"].as_str(),
        target.executable.to_str()
    );
    let claude: Value = serde_json::from_slice(&fs::read(&claude_file).unwrap()).unwrap();
    assert_eq!(claude["projects"]["workspace"]["existing"], true);
    assert_eq!(
        claude["mcpServers"]["onesdm-mcp"],
        json!({
            "type":"stdio", "command": target.executable,
            "args":["--mcp","--data-dir", target.profile],
        })
    );
    let pi: Value = serde_json::from_slice(&fs::read(&pi_file).unwrap()).unwrap();
    assert_eq!(pi["mcpServers"]["another"]["command"], "existing");
    assert_eq!(pi["mcpServers"]["onesdm-mcp"]["exposure"], "direct");
    let opencode = fs::read_to_string(&opencode_file).unwrap();
    assert!(opencode.contains("// Keep OpenCode preferences"));
    assert!(opencode.contains("\"type\": \"local\""));
    let dsh = fs::read_to_string(target.home.join(".dsh/cordis.patch.yml")).unwrap();
    assert!(dsh.contains("@deepseek-ai/dsh-mcp-client"));

    for (client, skill_root) in Harness::ALL.into_iter().zip(skill_roots) {
        client.remove(&target).unwrap();
        client.remove(&target).unwrap();
        assert_eq!(client.status(&target).unwrap(), Status::default());
        assert!(!target.home.join(skill_root).join("onesdm").exists());
    }
    assert!(
        fs::read_to_string(codex_file)
            .unwrap()
            .contains("other-model")
    );
    assert!(
        fs::read_to_string(claude_file)
            .unwrap()
            .contains("workspace")
    );
    assert!(fs::read_to_string(pi_file).unwrap().contains("existing"));
    assert!(
        fs::read_to_string(opencode_file)
            .unwrap()
            .contains("// Keep OpenCode preferences")
    );
}

#[test]
fn existing_managed_integrations_migrate_once_and_keep_their_settings() {
    let root = tempfile::tempdir().unwrap();
    let mut target = target(root.path());
    for (client, folder, config) in [
        (Harness::Codex, ".agents/skills", ".codex/config.toml"),
        (Harness::ClaudeCode, ".claude/skills", ".claude.json"),
        (Harness::Pi, ".pi/agent/skills", ".pi/agent/mcp.json"),
        (
            Harness::OpenCode,
            ".config/opencode/skills",
            ".config/opencode/opencode.jsonc",
        ),
        (Harness::DeepSeek, ".dsh/skills", ".dsh/cordis.patch.yml"),
    ] {
        client.install(&target).unwrap();
        let file = target.home.join(config);
        let legacy_config = fs::read_to_string(&file)
            .unwrap()
            .replace("onesdm-mcp", "ndm-next");
        fs::write(&file, &legacy_config).unwrap();
        let skills = target.home.join(folder);
        let old = skills.join("ndm-next");
        fs::rename(skills.join("onesdm"), &old).unwrap();
        fs::write(
            old.join("SKILL.md"),
            "<!-- Managed by the NDM Next desktop integration. -->",
        )
        .unwrap();
        fs::write(old.join("personal.md"), "my notes").unwrap();
        // Linux archives and relocated macOS bundles give the host a new path.
        let moved = root
            .path()
            .join(client.name())
            .join(target.executable.file_name().unwrap());
        fs::create_dir_all(moved.parent().unwrap()).unwrap();
        fs::rename(&target.executable, &moved).unwrap();
        target.executable = moved;
        for _ in 0..2 {
            ndm_host::harness::sync_skills(&target).unwrap();
        }
        assert!(client.status(&target).unwrap().installed());
        assert_eq!(
            fs::read_to_string(&file)
                .unwrap()
                .matches("ndm-next")
                .count(),
            0
        );
        assert!(fs::read_to_string(old.join("personal.md")).is_ok());
        assert!(!old.join("SKILL.md").exists());
    }
}

#[test]
fn a_legacy_codex_entry_can_be_repaired_after_moving_the_app() {
    let root = tempfile::tempdir().unwrap();
    let mut target = target(root.path());
    let path = target.home.join(".codex/config.toml");
    // The version that used "codex mcp add" wrote the same stdio entry without
    // any NDM-specific keys. Preserve settings the user added to it afterwards.
    let mut config = "[mcp_servers.onesdm-mcp]\nenabled = false\n"
        .parse::<DocumentMut>()
        .unwrap();
    let entry = &mut config["mcp_servers"]["onesdm-mcp"];
    entry["command"] = value(target.executable.to_str().unwrap());
    entry["args"] = value(
        ["--mcp", "--data-dir", target.profile.to_str().unwrap()]
            .into_iter()
            .collect::<Array>(),
    );
    entry["enabled_tools"] = value(["ndm_task_status"].into_iter().collect::<Array>());
    write(&path, &config.to_string());
    let new_location = root
        .path()
        .join("Applications")
        .join(target.executable.file_name().unwrap());
    fs::create_dir(new_location.parent().unwrap()).unwrap();
    fs::rename(&target.executable, &new_location).unwrap();
    target.executable = new_location;
    assert_eq!(
        Harness::Codex.status(&target).unwrap().mcp,
        McpStatus::NeedsRepair
    );
    Harness::Codex.install(&target).unwrap();
    assert!(Harness::Codex.status(&target).unwrap().installed());
    let config = fs::read_to_string(path)
        .unwrap()
        .parse::<DocumentMut>()
        .unwrap();
    let entry = &config["mcp_servers"]["onesdm-mcp"];
    assert_eq!(entry["command"].as_str(), target.executable.to_str());
    assert_eq!(entry["enabled"].as_bool(), Some(true));
    assert_eq!(
        entry["enabled_tools"]
            .as_array()
            .unwrap()
            .get(0)
            .unwrap()
            .as_str(),
        Some("ndm_task_status")
    );
}

#[test]
fn opencode_updates_existing_v2_jsonc_and_leaves_other_sources_in_place() {
    let root = tempfile::tempdir().unwrap();
    let target = target(root.path());
    let directory = target.home.join(".config/opencode");
    let other = directory.join("opencode.json");
    let config = directory.join("opencode.jsonc");
    let other_content = "{\"model\":\"vendor/example\"}\n";
    write(&other, other_content);
    write(
        &config,
        "{\n  \"mcp\": {\n    \"servers\": {\n      // Keep other servers\n      \"example\": {\"type\": \"remote\", \"url\": \"https://example.test/mcp\"},\n    },\n  },\n}\n",
    );
    Harness::OpenCode.install(&target).unwrap();
    assert!(Harness::OpenCode.status(&target).unwrap().installed());
    let contents = fs::read_to_string(&config).unwrap();
    let document = jsonc_parser::cst::CstRootNode::parse(&contents, &Default::default()).unwrap();
    let mcp = document
        .object_value()
        .unwrap()
        .object_value("mcp")
        .unwrap();
    let servers = mcp.object_value("servers").unwrap();
    assert!(mcp.get("onesdm-mcp").is_none());
    assert!(
        servers
            .object_value("onesdm-mcp")
            .unwrap()
            .get("codemode")
            .is_some()
    );
    assert!(servers.get("example").is_some());
    assert_eq!(fs::read_to_string(&other).unwrap(), other_content);
    Harness::OpenCode.remove(&target).unwrap();
    assert!(!fs::read_to_string(&config).unwrap().contains("onesdm-mcp"));
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("// Keep other servers")
    );
}

#[test]
fn failed_skill_install_restores_the_original_client_config() {
    let root = tempfile::tempdir().unwrap();
    let target = target(root.path());
    let codex = target.home.join(".codex/config.toml");
    let original = "# User preference\nmodel = \"example\"\n";
    write(&codex, original);
    fs::create_dir_all(target.profile.join("tasks.db")).unwrap();
    assert!(Harness::Codex.install(&target).is_err());
    assert_eq!(fs::read_to_string(&codex).unwrap(), original);
    assert_eq!(Harness::Codex.status(&target).unwrap(), Status::default());
}

#[test]
fn installing_or_removing_the_sdk_preserves_user_files() {
    let temp = tempfile::tempdir().unwrap();
    let target = target(temp.path());
    let config = target.home.join(".codex/config.toml");
    let original = "# Existing config\nmodel = \"example\"\n";
    write(&config, original);
    let root = target.home.join(".agents/skills/onesdm");
    let sdk = root.join("references/sdk");
    let readme = sdk.join("README.md");
    write(&readme, "User reference");
    assert_eq!(
        Harness::Codex.install(&target).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read_to_string(&readme).unwrap(), "User reference");
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert!(!root.join("SKILL.md").exists());
    fs::remove_file(&readme).unwrap();
    Harness::Codex.install(&target).unwrap();
    let manifest = sdk.join("examples/file-copy.json");
    write(&manifest, r#"{"id":"user-example"}"#);
    write(&sdk.join("notes.md"), "User notes");
    Harness::Codex.remove(&target).unwrap();
    assert_eq!(
        fs::read_to_string(&manifest).unwrap(),
        r#"{"id":"user-example"}"#
    );
    assert_eq!(
        fs::read_to_string(sdk.join("notes.md")).unwrap(),
        "User notes"
    );
    assert!(!readme.exists() && !root.join("SKILL.md").exists());
}

#[cfg(unix)]
#[test]
fn a_link_inside_the_sdk_is_never_followed() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let target = target(temp.path());
    Harness::Codex.install(&target).unwrap();
    let examples = target
        .home
        .join(".agents/skills/onesdm/references/sdk/examples");
    let outside = temp.path().join("original-examples");
    fs::rename(&examples, &outside).unwrap();
    symlink(&outside, &examples).unwrap();
    assert_eq!(
        ndm_host::harness::sync_skills(&target).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        Harness::Codex.remove(&target).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(outside.join("file-copy.rs").is_file());
}

#[test]
fn deepseek_preserves_other_patches_and_repairs_the_host_registration() {
    let directory = tempfile::tempdir().unwrap();
    // ':' is reserved in Windows file names. '#' after a space needs quoting in YAML.
    let segment = if cfg!(windows) {
        "an app #1"
    } else {
        "version: two #1"
    };
    let mut target = target(&directory.path().join(segment));
    let path = target.home.join(".dsh/cordis.patch.yml");
    let existing = "# upstream plugin options\n- id: existing\n  disabled: !!js \"!process.env.ENABLE_PLUGIN\"\n";
    write(&path, existing);
    Harness::DeepSeek.install(&target).unwrap();
    let before = fs::read_to_string(&path).unwrap();
    assert!(before.contains(existing));
    Harness::DeepSeek.install(&target).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    let moved = directory
        .path()
        .join(target.executable.file_name().unwrap());
    fs::rename(&target.executable, &moved).unwrap();
    target.executable = moved;
    assert_eq!(
        Harness::DeepSeek.status(&target).unwrap().mcp,
        McpStatus::NeedsRepair
    );
    Harness::DeepSeek.install(&target).unwrap();
    assert!(Harness::DeepSeek.status(&target).unwrap().installed());
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains(existing));
    Harness::DeepSeek.remove(&target).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), existing);
    Harness::DeepSeek.remove(&target).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), existing);
}

#[test]
fn deepseek_removes_only_the_managed_row_from_an_annotated_patch() {
    let directory = tempfile::tempdir().unwrap();
    let target = target(directory.path());
    let path = target.home.join(".dsh/cordis.patch.yml");
    let row = json!({
        "id": "mcp-onesdm-mcp", "name": "@deepseek-ai/dsh-mcp-client",
        "config": {"serverName": "onesdm-mcp", "transport": "stdio",
            "command": target.executable, "args": ["--mcp", "--data-dir", target.profile]}
    });
    for extra in [None, Some(json!({"name":"other", "id":"keep-this"}))] {
        let rows: Vec<_> = std::iter::once(row.clone()).chain(extra.clone()).collect();
        write(
            &path,
            &json!([{"insert":rows, "notes":"keep this note"}]).to_string(),
        );
        Harness::DeepSeek.remove(&target).unwrap();
        let file = fs::read_to_string(&path)
            .unwrap()
            .parse::<yaml_edit::Document>()
            .unwrap();
        let patch = file.as_sequence().unwrap().get(0).unwrap();
        let patch = patch.as_mapping().unwrap();
        assert_eq!(
            patch.get("notes").unwrap().as_scalar().unwrap().as_string(),
            "keep this note"
        );
        let entries = patch.get_sequence("insert").unwrap();
        assert_eq!(entries.len(), usize::from(extra.is_some()));
        if extra.is_some() {
            assert!(entries.to_string().contains("keep-this"));
        }
    }
}

#[test]
fn a_foreign_server_or_invalid_config_is_left_with_its_original_contents() {
    let root = tempfile::tempdir().unwrap();
    let target = target(root.path());
    for (client, file, previous, kind) in [
        (
            Harness::ClaudeCode,
            ".claude.json",
            "{\"mcpServers\":{\"onesdm-mcp\":{\"type\":\"http\",\"url\":\"https://example.test/mcp\"}}}",
            io::ErrorKind::AlreadyExists,
        ),
        (
            Harness::OpenCode,
            ".config/opencode/opencode.jsonc",
            "{\"mcp\": ",
            io::ErrorKind::InvalidData,
        ),
        (
            Harness::DeepSeek,
            ".dsh/cordis.patch.yml",
            "- insert:\n    - id: custom\n      name: '@deepseek-ai/dsh-mcp-client'\n      config: { serverName: onesdm-mcp, transport: stdio, command: other }\n",
            io::ErrorKind::AlreadyExists,
        ),
        (
            Harness::DeepSeek,
            ".dsh/cordis.patch.yml",
            "insert: {\n",
            io::ErrorKind::InvalidData,
        ),
        (
            Harness::DeepSeek,
            ".dsh/cordis.patch.yml",
            "- insert:\n    - id: tools\n      group: true\n      config:\n        - name: '@deepseek-ai/dsh-mcp-client'\n          config: {serverName: onesdm-mcp}\n",
            io::ErrorKind::AlreadyExists,
        ),
        (
            Harness::DeepSeek,
            ".dsh/cordis.patch.yml",
            "- id: tools\n  config:\n    - name: '@deepseek-ai/dsh-mcp-manager'\n      config: {servers: {onesdm-mcp: {command: other}}}\n",
            io::ErrorKind::AlreadyExists,
        ),
        (
            Harness::DeepSeek,
            ".dsh/cordis.patch.yml",
            "- id: custom\n  config: {serverName: onesdm-mcp, transport: stdio, command: other}\n",
            io::ErrorKind::AlreadyExists,
        ),
        (
            Harness::DeepSeek,
            ".dsh/cordis.patch.yml",
            "- id: custom\n  config: {serverName: &name onesdm-mcp}\n- insert:\n    - name: '@deepseek-ai/dsh-mcp-client'\n      config: {serverName: *name, transport: stdio, command: other}\n",
            io::ErrorKind::AlreadyExists,
        ),
    ] {
        let path = target.home.join(file);
        write(&path, previous);
        assert_eq!(client.status(&target).unwrap_err().kind(), kind);
        assert_eq!(client.install(&target).unwrap_err().kind(), kind);
        assert_eq!(client.remove(&target).unwrap_err().kind(), kind);
        assert_eq!(fs::read_to_string(path).unwrap(), previous);
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires Claude Code, Pi, and OpenCode on PATH and uses isolated user settings"]
fn native_clients_discover_the_onesdm_mcp_registration() {
    use std::{env, process::Command};

    let directory = tempfile::tempdir().unwrap();
    let target = Target::new(
        env!("CARGO_BIN_EXE_ndm-host").into(),
        directory.path().join("profile"),
        directory.path().join("user"),
    );
    for (client, command, args) in [
        (
            Harness::ClaudeCode,
            "claude",
            &["mcp", "get", "onesdm-mcp"][..],
        ),
        (Harness::Pi, "pi", &["mcp", "list"][..]),
        (Harness::OpenCode, "opencode", &["mcp", "list"][..]),
    ] {
        client.install(&target).unwrap();
        let mut process = Command::new(command);
        process
            .args(args)
            .current_dir(&target.home)
            .env_clear()
            .env("HOME", &target.home)
            .env("PATH", env::var_os("PATH").unwrap_or_default())
            .env("XDG_CONFIG_HOME", target.home.join(".config"))
            .env("XDG_DATA_HOME", target.home.join(".local/share"))
            .env("XDG_CACHE_HOME", target.home.join(".cache"))
            .env("OPENCODE_DISABLE_MODELS_FETCH", "1")
            .env("NO_COLOR", "1");
        if let Some(libraries) = env::var_os("LD_LIBRARY_PATH") {
            process.env("LD_LIBRARY_PATH", libraries);
        }
        let output = process.output().unwrap();
        assert!(
            output.status.success(),
            "{} rejected the registration",
            client.name()
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("onesdm-mcp"),
            "{} did not report the server",
            client.name(),
        );
        client.remove(&target).unwrap();
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires DeepSeek Harness on PATH and uses an isolated DSH_HOME"]
fn deepseek_composes_the_onesdm_mcp_client_across_profiles() {
    use std::{env, process::Command};

    let directory = tempfile::tempdir().unwrap();
    let target = target(directory.path());
    Harness::DeepSeek.install(&target).unwrap();
    for profile in ["web", "headless"] {
        let output = Command::new("dsh")
            .args(["--profile", profile, "--dump-config"])
            .current_dir(&target.home)
            .env_clear()
            .env("PATH", env::var_os("PATH").unwrap_or_default())
            .env("HOME", &target.home)
            .env("DSH_HOME", target.home.join(".dsh"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "dsh rejected the {profile} configuration"
        );
        let config = String::from_utf8(output.stdout).unwrap();
        assert!(
            config.contains("mcp-onesdm-mcp")
                && config.contains(target.executable.to_str().unwrap())
        );
    }
}
