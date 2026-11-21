use std::{
    io::{self, BufReader},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    DownloadRequest, ENGINE_PROTOCOL_VERSION, EngineMessage, FailureReason, HostMessage,
    SourceFiles, TaskUpdate, TransferProgress, read_message, write_message,
};

pub struct Worker {
    stopped: Arc<AtomicBool>,
    task_updates: bool,
    peer_stats: bool,
}

impl Worker {
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    pub fn progress(&self, progress: TransferProgress) -> io::Result<()> {
        write_message(
            &mut io::stdout().lock(),
            &EngineMessage::Progress { progress },
        )
    }

    pub fn update_task(&self, update: TaskUpdate) -> io::Result<()> {
        if self.task_updates && (!matches!(update, TaskUpdate::PeerStats(_)) || self.peer_stats) {
            write_message(
                &mut io::stdout().lock(),
                &EngineMessage::TaskUpdate { update },
            )?;
        }
        Ok(())
    }
}

/// Optional operations are only called when the installed manifest declares them.
pub trait Engine {
    fn download(
        &self,
        request: DownloadRequest,
        worker: &Worker,
    ) -> Result<Vec<String>, FailureReason>;

    fn inspect(&self, _: DownloadRequest, _: &Worker) -> Result<SourceFiles, FailureReason> {
        Err(FailureReason::Unsupported)
    }

    fn share(&self, _: DownloadRequest, _: PathBuf, _: &Worker) -> Result<(), FailureReason> {
        Err(FailureReason::Unsupported)
    }
}

pub fn serve_engine(engine: impl Engine) -> io::Result<()> {
    serve_request(|message, worker| match message {
        HostMessage::Download { request } => engine
            .download(*request, worker)
            .map(|files| EngineMessage::Completed { files }),
        HostMessage::Inspect { request } => engine
            .inspect(*request, worker)
            .map(|source| EngineMessage::Inspected { source }),
        HostMessage::Share { request, output } => engine
            .share(*request, output, worker)
            .map(|()| EngineMessage::Completed { files: Vec::new() }),
        _ => Err(FailureReason::Unsupported),
    })
}

/// Serve one download. Returning after a stop request confirms its checkpoint is ready.
pub fn serve(
    download: impl FnOnce(DownloadRequest, &Worker) -> Result<Vec<String>, FailureReason>,
) -> io::Result<()> {
    serve_request(|request, worker| match request {
        HostMessage::Download { request } => {
            download(*request, worker).map(|files| EngineMessage::Completed { files })
        }
        _ => Err(FailureReason::Unsupported),
    })
}

fn serve_request(
    handle: impl FnOnce(HostMessage, &Worker) -> Result<EngineMessage, FailureReason>,
) -> io::Result<()> {
    let mut input = BufReader::new(io::stdin());
    let (task_updates, peer_stats) = match read_message(&mut input)? {
        Some(HostMessage::Hello {
            version,
            task_updates,
            peer_stats,
        }) if version == ENGINE_PROTOCOL_VERSION => {
            write_message(&mut io::stdout().lock(), &EngineMessage::Ready { version })?;
            (task_updates, peer_stats)
        }
        _ => return Err(io::Error::other("unsupported engine handshake")),
    };
    let request = match read_message(&mut input)? {
        Some(
            request @ (HostMessage::Download { .. }
            | HostMessage::Inspect { .. }
            | HostMessage::Share { .. }),
        ) => request,
        Some(HostMessage::Stop) => {
            return write_message(&mut io::stdout().lock(), &EngineMessage::Stopped);
        }
        _ => return Err(io::Error::other("expected engine download request")),
    };
    let stopped = Arc::new(AtomicBool::new(false));
    let control = stopped.clone();
    std::thread::spawn(move || {
        let _: io::Result<Option<HostMessage>> = read_message(&mut input);
        control.store(true, Ordering::Release);
    });
    let worker = Worker {
        stopped,
        task_updates,
        peer_stats,
    };
    let result = handle(request, &worker);
    let outcome = if worker.stopped() {
        EngineMessage::Stopped
    } else {
        match result {
            Ok(message) => message,
            Err(FailureReason::InvalidOptions | FailureReason::PostProcessing) if !task_updates => {
                EngineMessage::Failed {
                    reason: FailureReason::Other,
                }
            }
            Err(reason) => EngineMessage::Failed { reason },
        }
    };
    write_message(&mut io::stdout().lock(), &outcome)
}
