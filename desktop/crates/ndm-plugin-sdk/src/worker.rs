use std::{
    io::{self, BufReader},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    DownloadRequest, ENGINE_PROTOCOL_VERSION, EngineMessage, FailureReason, HostMessage,
    TransferProgress, read_message, write_message,
};

pub struct Worker {
    stopped: Arc<AtomicBool>,
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
}

/// Serve one download. Returning after a stop request confirms its checkpoint is ready.
pub fn serve(
    download: impl FnOnce(DownloadRequest, &Worker) -> Result<Vec<String>, FailureReason>,
) -> io::Result<()> {
    let mut input = BufReader::new(io::stdin());
    match read_message(&mut input)? {
        Some(HostMessage::Hello { version }) if version == ENGINE_PROTOCOL_VERSION => {
            write_message(&mut io::stdout().lock(), &EngineMessage::Ready { version })?;
        }
        _ => return Err(io::Error::other("unsupported engine handshake")),
    }
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
    let worker = Worker { stopped };
    let result = download(request, &worker);
    let outcome = if worker.stopped() {
        EngineMessage::Stopped
    } else {
        match result {
            Ok(files) => EngineMessage::Completed { files },
            Err(reason) => EngineMessage::Failed { reason },
        }
    };
    write_message(&mut io::stdout().lock(), &outcome)
}
