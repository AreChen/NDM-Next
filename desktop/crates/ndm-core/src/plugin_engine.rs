//! On-demand process host for plugin download engines.

mod process;

use std::{
    collections::HashSet,
    io,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use ndm_plugin_sdk::{
    DownloadRequest, ENGINE_ARGUMENT, ENGINE_PROTOCOL_VERSION, EngineMessage, FailureReason,
    HostMessage, MAX_MESSAGE_BYTES, PeerStats, SourceFiles, TaskUpdate, TransferProgress,
    encode_message,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("could not run the download engine: {0}")]
    Io(#[from] io::Error),
    #[error("download engine sent an invalid message")]
    Protocol,
    #[error("download engine did not accept the request")]
    StartTimeout,
    #[error("download engine did not stop before the deadline")]
    StopTimeout,
    #[error("download engine returned an invalid output")]
    InvalidOutput,
    #[error("download engine failed: {0:?}")]
    Failed(FailureReason),
}

#[derive(Debug, PartialEq, Eq)]
pub enum EngineOutcome {
    Completed(Vec<PathBuf>),
    Stopped,
}

pub(crate) enum EngineUpdate {
    Progress(TransferProgress, Option<u64>),
    Task(TaskUpdate),
}

#[derive(Clone)]
pub struct PluginEngine {
    executable: PathBuf,
    pub(crate) runtimes: std::sync::Arc<crate::plugins::ResolvedRuntimes>,
}

impl PluginEngine {
    /// The executable comes from the host's installed plugin, never a task.
    pub fn new(executable: PathBuf) -> io::Result<Self> {
        if !executable.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "download engine path must be absolute",
            ));
        }
        let metadata = std::fs::symlink_metadata(&executable)?;
        if !metadata.is_file() || crate::cache::is_reparse(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "download engine is not a regular file",
            ));
        }
        Ok(Self {
            executable,
            runtimes: Default::default(),
        })
    }

    /// Run one transfer and reclaim its process tree before returning.
    pub async fn download(
        &self,
        request: DownloadRequest,
        cancel: &CancellationToken,
        mut progress: impl FnMut(TransferProgress),
    ) -> Result<EngineOutcome, EngineError> {
        self.download_with_updates(request, cancel, |update| {
            if let EngineUpdate::Progress(update, _) = update {
                progress(update);
            }
        })
        .await
    }

    pub(crate) async fn download_with_updates(
        &self,
        request: DownloadRequest,
        cancel: &CancellationToken,
        update: impl FnMut(EngineUpdate),
    ) -> Result<EngineOutcome, EngineError> {
        let workspace = request.workspace.clone();
        let command = HostMessage::Download {
            request: Box::new(request),
        };
        match self.run(&workspace, command, cancel, update).await? {
            Terminal::Completed(files) => {
                Ok(EngineOutcome::Completed(output_paths(&workspace, files)?))
            }
            Terminal::Stopped => Ok(EngineOutcome::Stopped),
            Terminal::Inspected(_) => Err(EngineError::Protocol),
        }
    }

    pub(crate) async fn inspect(
        &self,
        request: DownloadRequest,
        cancel: &CancellationToken,
    ) -> Result<Option<SourceFiles>, EngineError> {
        let workspace = request.workspace.clone();
        let command = HostMessage::Inspect {
            request: Box::new(request),
        };
        match self.run(&workspace, command, cancel, |_| {}).await? {
            Terminal::Inspected(source) if source.valid() => Ok(Some(source)),
            Terminal::Stopped => Ok(None),
            _ => Err(EngineError::Protocol),
        }
    }

    pub(crate) async fn share(
        &self,
        request: DownloadRequest,
        output: PathBuf,
        cancel: &CancellationToken,
        mut report: impl FnMut(PeerStats),
    ) -> Result<(), EngineError> {
        let workspace = request.workspace.clone();
        let command = HostMessage::Share {
            request: Box::new(request),
            output,
        };
        match self
            .run(&workspace, command, cancel, |update| {
                if let EngineUpdate::Task(TaskUpdate::PeerStats(stats)) = update {
                    report(stats);
                }
            })
            .await?
        {
            Terminal::Stopped => Ok(()),
            Terminal::Completed(files) if files.is_empty() => Ok(()),
            _ => Err(EngineError::Protocol),
        }
    }

    async fn run(
        &self,
        workspace: &Path,
        request: HostMessage,
        cancel: &CancellationToken,
        mut update: impl FnMut(EngineUpdate),
    ) -> Result<Terminal, EngineError> {
        if cancel.is_cancelled() {
            return Ok(Terminal::Stopped);
        }
        if !workspace.is_absolute() {
            return Err(EngineError::InvalidOutput);
        }
        crate::cache::validate_directory(workspace)?;

        let installed_directory = self.executable.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing download engine directory",
            )
        })?;
        let mut command = Command::new(&self.executable);
        command.env_remove(ndm_plugin_sdk::RUNTIMES_ENV);
        if !self.runtimes.paths.is_empty() {
            command.env(
                ndm_plugin_sdk::RUNTIMES_ENV,
                serde_json::to_string(&self.runtimes.paths).map_err(io::Error::other)?,
            );
        }
        command
            .arg(ENGINE_ARGUMENT)
            .current_dir(installed_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(tools) = crate::media::LocalTools::installed_directory() {
            command.env(ndm_plugin_sdk::MEDIA_TOOLS_ENV, tools);
        }
        let mut worker = process::Worker::spawn(&mut command)?;
        let mut channel = Channel {
            input: worker.child.stdin.take().ok_or(EngineError::Protocol)?,
            output: BufReader::new(worker.child.stdout.take().ok_or(EngineError::Protocol)?),
            line: Vec::new(),
        };
        let outcome = channel.transfer(request, cancel, &mut update).await;
        drop(channel);
        let reaped = worker.reap().await;
        let outcome = outcome?;
        reaped?;

        Ok(outcome)
    }
}

struct Channel {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    line: Vec<u8>,
}

enum Terminal {
    Completed(Vec<String>),
    Inspected(SourceFiles),
    Stopped,
}

impl Channel {
    async fn send(&mut self, message: &HostMessage) -> Result<(), EngineError> {
        self.input.write_all(&encode_message(message)?).await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<EngineMessage, EngineError> {
        // read_until preserves the bytes already consumed if cancellation
        // interrupts a read. Continue the same frame after sending Stop.
        let remaining = MAX_MESSAGE_BYTES + 1 - self.line.len();
        (&mut self.output)
            .take(remaining as u64)
            .read_until(b'\n', &mut self.line)
            .await?;
        if self.line.pop() != Some(b'\n') || self.line.len() > MAX_MESSAGE_BYTES {
            return Err(EngineError::Protocol);
        }
        let result = serde_json::from_slice(&self.line).map_err(|_| EngineError::Protocol);
        self.line.clear();
        result
    }

    async fn start(&mut self, request: HostMessage) -> Result<(), EngineError> {
        self.send(&HostMessage::Hello {
            version: ENGINE_PROTOCOL_VERSION,
            task_updates: true,
            peer_stats: true,
        })
        .await?;
        match self.receive().await? {
            EngineMessage::Ready { version } if version == ENGINE_PROTOCOL_VERSION => {
                self.send(&request).await
            }
            _ => Err(EngineError::Protocol),
        }
    }

    async fn terminal(
        &mut self,
        update: &mut impl FnMut(EngineUpdate),
    ) -> Result<Terminal, EngineError> {
        loop {
            match self.receive().await? {
                EngineMessage::Progress {
                    progress,
                    estimated_total,
                } => update(EngineUpdate::Progress(progress, estimated_total)),
                EngineMessage::TaskUpdate { update: task } => update(EngineUpdate::Task(task)),
                EngineMessage::Completed { files } => return Ok(Terminal::Completed(files)),
                EngineMessage::Inspected { source } => return Ok(Terminal::Inspected(source)),
                EngineMessage::Stopped => return Ok(Terminal::Stopped),
                EngineMessage::Failed { reason } => return Err(EngineError::Failed(reason)),
                EngineMessage::Ready { .. } => return Err(EngineError::Protocol),
            }
        }
    }

    async fn transfer(
        &mut self,
        request: HostMessage,
        cancel: &CancellationToken,
        update: &mut impl FnMut(EngineUpdate),
    ) -> Result<Terminal, EngineError> {
        tokio::select! {
            // Once Download is sent, stop via the normal checkpoint path.
            biased;
            started = timeout(START_TIMEOUT, self.start(request)) => {
                started.map_err(|_| EngineError::StartTimeout)??;
            }
            _ = cancel.cancelled() => return Ok(Terminal::Stopped),
        }
        tokio::select! {
            result = self.terminal(update) => return result,
            _ = cancel.cancelled() => {}
        }
        timeout(STOP_TIMEOUT, async {
            // Completion may already be in stdout when the worker closes stdin.
            let _ = self.send(&HostMessage::Stop).await;
            self.terminal(update).await
        })
        .await
        .map_err(|_| EngineError::StopTimeout)?
    }
}

fn output_paths(workspace: &Path, files: Vec<String>) -> Result<Vec<PathBuf>, EngineError> {
    if files.is_empty() {
        return Err(EngineError::InvalidOutput);
    }
    crate::cache::validate_directory(workspace)?;
    let mut seen = HashSet::new();
    files
        .into_iter()
        .map(|file| {
            let mut path = workspace.to_path_buf();
            let mut parts = file.split('/').peekable();
            while let Some(part) = parts.next() {
                if part.is_empty()
                    || part.contains(['\\', ':', '\0'])
                    || !matches!(
                        Path::new(part).components().next(),
                        Some(Component::Normal(_))
                    )
                {
                    return Err(EngineError::InvalidOutput);
                }
                path.push(part);
                let metadata =
                    std::fs::symlink_metadata(&path).map_err(|_| EngineError::InvalidOutput)?;
                if crate::cache::is_reparse(&metadata)
                    || if parts.peek().is_some() {
                        !metadata.is_dir()
                    } else {
                        !metadata.is_file()
                    }
                {
                    return Err(EngineError::InvalidOutput);
                }
            }
            if !seen.insert(path.canonicalize()?) {
                return Err(EngineError::InvalidOutput);
            }
            Ok(path)
        })
        .collect()
}
