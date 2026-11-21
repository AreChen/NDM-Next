use std::{
    io::{self, BufReader},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    DownloadRequest, ENGINE_PROTOCOL_VERSION, EngineMessage, FailureReason, HostMessage,
    TaskUpdate, TransferProgress, read_message, write_message,
};

pub struct Worker {
    stopped: Arc<AtomicBool>,
    task_updates: bool,
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
        if self.task_updates {
            write_message(
                &mut io::stdout().lock(),
                &EngineMessage::TaskUpdate { update },
            )?;
        }
        Ok(())
    }
}

/// Serve one download. Returning after a stop request confirms its checkpoint is ready.
pub fn serve(
    download: impl FnOnce(DownloadRequest, &Worker) -> Result<Vec<String>, FailureReason>,
) -> io::Result<()> {
    let mut input = BufReader::new(io::stdin());
    let task_updates = match read_message(&mut input)? {
        Some(HostMessage::Hello {
            version,
            task_updates,
        }) if version == ENGINE_PROTOCOL_VERSION => {
            write_message(&mut io::stdout().lock(), &EngineMessage::Ready { version })?;
            task_updates
        }
        _ => return Err(io::Error::other("unsupported engine handshake")),
    };
    let request = match read_message(&mut input)? {
        Some(HostMessage::Download { request }) => *request,
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
    };
    let result = download(request, &worker);
    let outcome = if worker.stopped() {
        EngineMessage::Stopped
    } else {
        match result {
            Ok(files) => EngineMessage::Completed { files },
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
