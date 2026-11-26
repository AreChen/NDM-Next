use std::{
    env, fs, io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use ndm_core::{
    BatchLimit, NewTask, PeerStats, PluginOptions, Scheduler, SchedulerOptions, SharingStatus,
    Store, TaskId, TaskOptions, TaskPolicy, TaskState,
    plugin_engine::{EngineError, EngineOutcome, PluginEngine},
    plugins::PluginRegistry,
};
use ndm_plugin_sdk::{
    DownloadRequest, DownloadSource, ENGINE_ARGUMENT, ENGINE_PROTOCOL_VERSION, EngineMessage,
    HostMessage, SourceFile, SourceFiles, SourceSelection, TaskUpdate, TransferProgress,
    read_message, write_message,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const TEST_TIMEOUT: Duration = Duration::from_secs(15);
const DESCENDANT_ARGUMENT: &str = "--test-engine-descendant";

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some(ENGINE_ARGUMENT) => fixture_engine(),
        Some("--fixture-runtime-version") => println!("fixture runtime 1"),
        Some(DESCENDANT_ARGUMENT) => {
            let _connection = std::net::TcpStream::connect(args.next().unwrap()).unwrap();
            loop {
                std::thread::park();
            }
        }
        _ => run_tests(),
    }
}

fn fixture_engine() {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    assert!(matches!(
        read_message::<HostMessage>(&mut input).unwrap(),
        Some(HostMessage::Hello { version, task_updates: true, .. }) if version == ENGINE_PROTOCOL_VERSION
    ));
    write_message(
        &mut output,
        &EngineMessage::Ready {
            version: ENGINE_PROTOCOL_VERSION,
        },
    )
    .unwrap();
    let request = match read_message(&mut input).unwrap().unwrap() {
        HostMessage::Download { request } => request,
        HostMessage::Inspect { request } => {
            if request.options.get("mode").and_then(|v| v.as_str()) == Some("shared-tool") {
                let tool = ndm_plugin_sdk::runtime_path("tool").expect("installed runtime");
                assert_ne!(tool, env::current_exe().unwrap());
                let result = Command::new(tool)
                    .arg("--fixture-runtime-version")
                    .output()
                    .unwrap();
                assert!(result.status.success());
                assert_eq!(
                    std::str::from_utf8(&result.stdout).unwrap().trim(),
                    "fixture runtime 1"
                );
            }
            write_message(
                &mut output,
                &EngineMessage::Inspected {
                    source: SourceFiles {
                        revision: "fixture-v1".into(),
                        files: vec![SourceFile {
                            id: 0,
                            path: "selected.bin".into(),
                            size: 8,
                        }],
                    },
                },
            )
            .unwrap();
            return;
        }
        HostMessage::Share {
            request,
            output: published,
        } => {
            assert_eq!(fs::read(published).unwrap(), b"selected");
            assert!(request.workspace.join("state/metadata").is_file());
            write_message(
                &mut output,
                &EngineMessage::TaskUpdate {
                    update: TaskUpdate::PeerStats(PeerStats {
                        connected: 2,
                        seeders: 1,
                        uploaded: 16,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();
            assert!(matches!(
                read_message::<HostMessage>(&mut input).unwrap(),
                Some(HostMessage::Stop)
            ));
            write_message(&mut output, &EngineMessage::Stopped).unwrap();
            return;
        }
        _ => panic!("unexpected fixture request"),
    };
    match request
        .options
        .get("mode")
        .and_then(serde_json::Value::as_str)
    {
        Some("complete") => {
            fs::create_dir(request.workspace.join("media")).unwrap();
            fs::write(request.workspace.join("media/clip.mp4"), b"clip").unwrap();
            fs::write(request.workspace.join("notes.txt"), b"notes").unwrap();
            write_message(
                &mut output,
                &EngineMessage::Progress {
                    progress: TransferProgress {
                        downloaded: 9,
                        total: Some(9),
                        transferred: 9,
                    },
                    estimated_total: None,
                },
            )
            .unwrap();
            write_message(
                &mut output,
                &EngineMessage::Completed {
                    files: vec!["media/clip.mp4".into(), "notes.txt".into()],
                },
            )
            .unwrap();
        }
        Some("invalid") => {
            write_message(
                &mut output,
                &EngineMessage::Completed {
                    files: vec!["../outside.bin".into()],
                },
            )
            .unwrap();
        }
        Some("selected") => {
            assert_eq!(request.selection.as_ref().unwrap().file_ids, vec![0]);
            fs::create_dir(request.workspace.join("state")).unwrap();
            fs::write(request.workspace.join("state/metadata"), b"fixture").unwrap();
            fs::write(request.workspace.join("selected.bin"), b"selected").unwrap();
            write_message(
                &mut output,
                &EngineMessage::Completed {
                    files: vec!["selected.bin".into()],
                },
            )
            .unwrap();
        }
        Some("stop") => {
            let mut descendant = Command::new(env::current_exe().unwrap())
                .arg(DESCENDANT_ARGUMENT)
                .arg(request.options["address"].as_str().unwrap())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            assert!(matches!(
                read_message::<HostMessage>(&mut input).unwrap(),
                Some(HostMessage::Stop)
            ));
            fs::write(request.workspace.join("checkpoint"), b"ready").unwrap();
            write_message(&mut output, &EngineMessage::Stopped).unwrap();
            let _ = descendant.wait();
        }
        Some("scheduled") => {
            use std::io::{Read, Write};
            write_message(
                &mut output,
                &EngineMessage::TaskUpdate {
                    update: TaskUpdate::Title {
                        title: "测试视频".into(),
                    },
                },
            )
            .unwrap();
            let mut gate =
                std::net::TcpStream::connect(request.options["address"].as_str().unwrap()).unwrap();
            gate.write_all(request.task_id.as_bytes()).unwrap();
            gate.write_all(&[request.download_threads.unwrap().get() as u8])
                .unwrap();
            gate.read_exact(&mut [0]).unwrap();
            fs::write(request.workspace.join("output.bin"), b"scheduled fixture").unwrap();
            write_message(
                &mut output,
                &EngineMessage::Completed {
                    files: vec!["output.bin".into()],
                },
            )
            .unwrap();
        }
        _ => panic!("unexpected fixture mode"),
    }
    // A terminal reply ends the job even when an executable stays alive.
    loop {
        std::thread::park();
    }
}

fn request(workspace: &Path, mode: &str) -> DownloadRequest {
    DownloadRequest {
        task_id: uuid::Uuid::new_v4(),
        source: DownloadSource::Url {
            url: "fixture://download".into(),
            headers: Default::default(),
        },
        workspace: workspace.to_owned(),
        suggested_filename: None,
        expected_size: None,
        download_threads: None,
        options: [("mode".into(), mode.into())].into_iter().collect(),
        selection: None,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn run_tests() {
    let engine = PluginEngine::new(env::current_exe().unwrap()).unwrap();
    completes_and_reaps_worker(&engine).await;
    rejects_outputs_outside_workspace(&engine).await;
    stops_and_reaps_descendant(&engine).await;
    schedules_installed_engines_with_global_batch_and_plugin_limits().await;
    inspects_selected_outputs_and_restarts_sharing_without_touching_files().await;
    invokes_an_installed_provider_from_a_consumer_worker().await;
    println!("6 download engine process and scheduler tests passed");
}

fn fixture_package(profile: &Path, id: &str, active: u32) -> std::path::PathBuf {
    let archive = profile.join(format!("{id}.zip"));
    let executable = format!("fixture-engine{}", env::consts::EXE_SUFFIX);
    let file = fs::File::create(&archive).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("plugin.json", options).unwrap();
    let mut manifest = serde_json::json!({
        "schema":1,"id":id,"version":"0.1.0","engine":executable,"engine_protocol":1,
        "name":{"en":id},"description":{"en":"Local download fixture"},
        "inputs":[{"schemes":["fixture"]}],"limits":{"active_tasks":active,"download_threads":4},
        "capabilities":{"inspect_files":true,"share_completed":true},
        "options":[
          {"key":"mode","kind":"text","max_length":32,"label":{"en":"Mode"}},
          {"key":"address","kind":"text","max_length":64,"label":{"en":"Local fixture address"}}
        ]
    });
    if id == "runtime-consumer" {
        manifest["schema"] = 2.into();
        manifest["runtime_dependencies"] = serde_json::json!([
            {"name":"tool","plugin":"runtime-provider","version":"^0.1","path":executable}
        ]);
    }
    serde_json::to_writer(&mut zip, &manifest).unwrap();
    zip.start_file(&executable, options.unix_permissions(0o755))
        .unwrap();
    io::copy(
        &mut fs::File::open(env::current_exe().unwrap()).unwrap(),
        &mut zip,
    )
    .unwrap();
    zip.finish().unwrap();
    archive
}

async fn invokes_an_installed_provider_from_a_consumer_worker() {
    let dir = tempfile::tempdir().unwrap();
    let registry = PluginRegistry::new(dir.path());
    registry
        .import(&fixture_package(dir.path(), "runtime-provider", 1))
        .unwrap();
    let consumer = registry
        .import(&fixture_package(dir.path(), "runtime-consumer", 1))
        .unwrap();
    let response = consumer
        .inspect_source(
            reqwest::Url::parse("fixture://shared").unwrap(),
            [("mode".into(), "shared-tool".into())]
                .into_iter()
                .collect(),
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.files[0].path, "selected.bin");
}

async fn inspects_selected_outputs_and_restarts_sharing_without_touching_files() {
    let profile = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(Store::open(&profile.path().join("tasks.db")).unwrap());
    let plugin = PluginRegistry::new(profile.path())
        .import(&fixture_package(profile.path(), "share-fixture", 1))
        .unwrap();
    let source = reqwest::Url::parse("fixture://fixture.invalid/download").unwrap();
    let listing = plugin
        .inspect_source(
            source.clone(),
            Default::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listing.files[0].path, "selected.bin");
    let id = TaskId::new_v4();
    store
        .enqueue(&[(
            id,
            NewTask {
                url: source.into(),
                destination: profile.path().join("downloads"),
                suggested_filename: None,
                source_url: None,
                expected_size: None,
                headers: Default::default(),
            },
            TaskOptions::Plugin(PluginOptions {
                id: plugin.manifest.id,
                values: [("mode".into(), "selected".into())].into_iter().collect(),
                selection: Some(SourceSelection {
                    revision: listing.revision,
                    file_ids: vec![0],
                }),
            }),
            TaskPolicy::default(),
        )])
        .unwrap();
    let scheduler = Scheduler::start(store.clone(), SchedulerOptions::default()).unwrap();
    timeout(TEST_TIMEOUT, async {
        loop {
            if store.get(id).unwrap().unwrap().state == TaskState::Completed
                && scheduler.progress(id).is_none()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let task = store.get(id).unwrap().unwrap();
    assert!(scheduler.can_share(&task));
    assert!(
        task.work_directory()
            .join("engine/state/metadata")
            .is_file()
    );
    assert!(!task.work_directory().join("engine/selected.bin").exists());
    for uploaded in [16, 32] {
        scheduler.start_sharing(id).unwrap();
        timeout(TEST_TIMEOUT, async {
            while scheduler.sharing_status(id) != Some(SharingStatus::Sharing) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let peers = scheduler.progress(id).unwrap().peers.unwrap();
        assert_eq!(
            (peers.connected, peers.seeders, peers.uploaded),
            (2, 1, uploaded)
        );
        assert!(scheduler.delete(id, true).is_err());
        assert!(scheduler.uninstall_plugin("share-fixture").is_err());
        scheduler.stop_sharing(id).await;
        assert!(scheduler.sharing_status(id).is_none());
        let saved = store.get(id).unwrap().unwrap();
        assert_eq!(saved.state, TaskState::Completed);
        assert_eq!(
            saved.progress.unwrap().peers.unwrap(),
            PeerStats {
                uploaded,
                ..Default::default()
            }
        );
        assert_eq!(fs::read(saved.output_path.unwrap()).unwrap(), b"selected");
    }
    scheduler.start_sharing(id).unwrap();
    scheduler.shutdown().await;
    assert!(scheduler.sharing_status(id).is_none());
    scheduler.uninstall_plugin("share-fixture").unwrap();
}

async fn schedules_installed_engines_with_global_batch_and_plugin_limits() {
    let profile = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(Store::open(&profile.path().join("tasks.db")).unwrap());
    let registry = PluginRegistry::new(profile.path());
    registry
        .import(&fixture_package(profile.path(), "engine-a", 1))
        .unwrap();
    registry
        .import(&fixture_package(profile.path(), "engine-b", 2))
        .unwrap();
    let source = reqwest::Url::parse("fixture://fixture.invalid/download").unwrap();
    registry.remember(&source, "engine-b").unwrap();
    assert_eq!(
        registry.candidates(&source).unwrap()[0].manifest.id,
        "engine-b"
    );
    let gate = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = gate.local_addr().unwrap().to_string();
    let batch = BatchLimit {
        id: TaskId::new_v4(),
        active_tasks: 1,
    };
    let items: Vec<_> = [
        ("engine-a", Some(batch)),
        ("engine-a", None),
        ("engine-b", Some(batch)),
        ("engine-b", None),
    ]
    .into_iter()
    .map(|(id, batch)| {
        let options = serde_json::json!({"mode":"scheduled","address":address});
        (
            TaskId::new_v4(),
            NewTask {
                url: source.to_string(),
                destination: profile.path().join("downloads"),
                suggested_filename: None,
                source_url: None,
                expected_size: None,
                headers: Default::default(),
            },
            TaskOptions::Plugin(PluginOptions {
                id: id.to_owned(),
                values: options.as_object().unwrap().clone(),
                selection: None,
            }),
            TaskPolicy {
                batch,
                download_threads: Some(12),
                ..Default::default()
            },
        )
    })
    .collect();
    store.enqueue(&items).unwrap();
    let scheduler = Scheduler::start(
        store.clone(),
        SchedulerOptions {
            max_active_tasks: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let mut active = std::collections::HashMap::new();
    for _ in 0..2 {
        let (mut stream, _) = timeout(TEST_TIMEOUT, gate.accept()).await.unwrap().unwrap();
        let mut admission = [0; 17];
        stream.read_exact(&mut admission).await.unwrap();
        assert_eq!(admission[16], 4);
        active.insert(TaskId::from_slice(&admission[..16]).unwrap(), stream);
    }
    assert!(active.contains_key(&items[0].0) && active.contains_key(&items[3].0));
    assert!(scheduler.uninstall_plugin("engine-a").is_err());
    assert!(
        timeout(Duration::from_millis(150), gate.accept())
            .await
            .is_err()
    );
    active
        .remove(&items[0].0)
        .unwrap()
        .write_all(b"r")
        .await
        .unwrap();
    let (mut next, _) = timeout(TEST_TIMEOUT, gate.accept()).await.unwrap().unwrap();
    let mut admission = [0; 17];
    next.read_exact(&mut admission).await.unwrap();
    assert_eq!(TaskId::from_slice(&admission[..16]).unwrap(), items[1].0);
    next.write_all(b"r").await.unwrap();
    active
        .remove(&items[3].0)
        .unwrap()
        .write_all(b"r")
        .await
        .unwrap();
    let (mut last, _) = timeout(TEST_TIMEOUT, gate.accept()).await.unwrap().unwrap();
    last.read_exact(&mut admission).await.unwrap();
    assert_eq!(TaskId::from_slice(&admission[..16]).unwrap(), items[2].0);
    last.write_all(b"r").await.unwrap();
    timeout(TEST_TIMEOUT, async {
        loop {
            if store
                .list()
                .unwrap()
                .iter()
                .all(|task| task.state == TaskState::Completed)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    scheduler.shutdown().await;
    scheduler.uninstall_plugin("engine-a").unwrap();
    for task in store.list().unwrap() {
        assert!(!task.work_directory().exists());
        assert_eq!(task.engine_title.as_deref(), Some("测试视频"));
        assert_eq!(
            store.get(task.id).unwrap().unwrap().engine_title,
            task.engine_title
        );
        assert_eq!(
            fs::read(task.output_path.unwrap()).unwrap(),
            b"scheduled fixture"
        );
    }
    assert_eq!(
        fs::read_dir(profile.path().join("downloads"))
            .unwrap()
            .count(),
        4
    );
}

async fn completes_and_reaps_worker(engine: &PluginEngine) {
    let dir = tempfile::tempdir().unwrap();
    let mut progress = Vec::new();
    let result = timeout(
        TEST_TIMEOUT,
        engine.download(
            request(dir.path(), "complete"),
            &CancellationToken::new(),
            |update| progress.push(update),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let EngineOutcome::Completed(files) = result else {
        panic!("expected finished files");
    };
    assert_eq!(
        files,
        [
            dir.path().join("media/clip.mp4"),
            dir.path().join("notes.txt")
        ]
    );
    assert_eq!(fs::read(&files[0]).unwrap(), b"clip");
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].downloaded, 9);
}

async fn rejects_outputs_outside_workspace(engine: &PluginEngine) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("work");
    fs::create_dir(&workspace).unwrap();
    fs::write(dir.path().join("outside.bin"), b"do not deliver").unwrap();
    let result = timeout(
        TEST_TIMEOUT,
        engine.download(
            request(&workspace, "invalid"),
            &CancellationToken::new(),
            |_| {},
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(EngineError::InvalidOutput)));
}

async fn stops_and_reaps_descendant(engine: &PluginEngine) {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut request = request(dir.path(), "stop");
    request.options.insert(
        "address".into(),
        listener.local_addr().unwrap().to_string().into(),
    );
    let cancel = CancellationToken::new();
    let task = {
        let engine = engine.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { engine.download(request, &cancel, |_| {}).await })
    };
    let (mut connection, _) = timeout(TEST_TIMEOUT, listener.accept())
        .await
        .unwrap()
        .unwrap();
    cancel.cancel();
    let result = timeout(TEST_TIMEOUT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(result, EngineOutcome::Stopped);
    assert_eq!(fs::read(dir.path().join("checkpoint")).unwrap(), b"ready");
    assert_connection_closed(&mut connection).await;
}

async fn assert_connection_closed(connection: &mut tokio::net::TcpStream) {
    match timeout(TEST_TIMEOUT, connection.read(&mut [0])).await {
        Ok(Ok(0)) => {}
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionReset => {}
        result => panic!("engine descendant remained active: {result:?}"),
    }
}
