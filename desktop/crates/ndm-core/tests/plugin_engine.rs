use std::{
    env, fs, io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use ndm_core::{
    BatchLimit, NewTask, PluginOptions, Scheduler, SchedulerOptions, Store, TaskId, TaskOptions,
    TaskPolicy, TaskState,
    plugin_engine::{EngineError, EngineOutcome, PluginEngine},
    plugins::PluginRegistry,
};
use ndm_plugin_sdk::{
    DownloadRequest, DownloadSource, ENGINE_ARGUMENT, ENGINE_PROTOCOL_VERSION, EngineMessage,
    HostMessage, TransferProgress, read_message, write_message,
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
        Some(HostMessage::Hello { version }) if version == ENGINE_PROTOCOL_VERSION
    ));
    write_message(
        &mut output,
        &EngineMessage::Ready {
            version: ENGINE_PROTOCOL_VERSION,
        },
    )
    .unwrap();
    let HostMessage::Download { request } = read_message(&mut input).unwrap().unwrap() else {
        panic!("expected a download request");
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
    }
}

#[tokio::main(flavor = "current_thread")]
async fn run_tests() {
    let engine = PluginEngine::new(env::current_exe().unwrap()).unwrap();
    completes_and_reaps_worker(&engine).await;
    rejects_outputs_outside_workspace(&engine).await;
    stops_and_reaps_descendant(&engine).await;
    schedules_installed_engines_with_global_batch_and_plugin_limits().await;
    println!("4 download engine process and scheduler tests passed");
}

fn fixture_package(profile: &Path, id: &str, active: u32) -> std::path::PathBuf {
    let archive = profile.join(format!("{id}.zip"));
    let executable = format!("fixture-engine{}", env::consts::EXE_SUFFIX);
    let file = fs::File::create(&archive).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("plugin.json", options).unwrap();
    serde_json::to_writer(&mut zip, &serde_json::json!({
        "schema":1,"id":id,"version":"0.1.0","engine":executable,"engine_protocol":1,
        "name":{"en":id},"description":{"en":"Local download fixture"},
        "inputs":[{"schemes":["fixture"]}],"limits":{"active_tasks":active,"download_threads":4},
        "options":[
          {"key":"mode","kind":"text","max_length":32,"label":{"en":"Mode"}},
          {"key":"address","kind":"text","max_length":64,"label":{"en":"Local fixture address"}}
        ]
    })).unwrap();
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
            }),
            TaskPolicy {
                batch,
                download_threads: Some(12),
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
