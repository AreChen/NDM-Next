use std::{
    env, fs, io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use ndm_core::plugin_engine::{EngineError, EngineOutcome, PluginEngine};
use ndm_plugin_sdk::{
    DownloadRequest, DownloadSource, ENGINE_ARGUMENT, ENGINE_PROTOCOL_VERSION, EngineMessage,
    HostMessage, TransferProgress, read_message, write_message,
};
use tokio::{io::AsyncReadExt, net::TcpListener, time::timeout};
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
    println!("3 download engine process tests passed");
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
