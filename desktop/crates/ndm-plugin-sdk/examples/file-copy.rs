use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
};

use ndm_plugin_sdk::{
    DownloadRequest, DownloadSource, FailureReason, TransferProgress, Worker, serve,
};

fn main() -> io::Result<()> {
    serve(|request, worker| copy(request, worker).map_err(|_| FailureReason::Storage))
}

fn copy(request: DownloadRequest, worker: &Worker) -> io::Result<Vec<String>> {
    let DownloadSource::File { path } = request.source else {
        return Err(io::Error::other("the file-copy engine takes a local file"));
    };
    let mut source = fs::File::open(path)?;
    let total = source.metadata()?.len();
    let result = request.workspace.join("copy.bin");
    if result.is_file() {
        return Ok(vec!["copy.bin".into()]);
    }
    let partial = request.workspace.join("copy.part");
    let mut output = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&partial)?;
    let mut downloaded = output.metadata()?.len();
    if downloaded > total {
        return Err(io::Error::other("source and checkpoint differ"));
    }
    source.seek(SeekFrom::Start(downloaded))?;
    let mut buffer = [0u8; 64 * 1024];
    while !worker.stopped() {
        worker.progress(TransferProgress {
            downloaded,
            total: Some(total),
            transferred: 0,
        })?;
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        downloaded += count as u64;
    }
    output.sync_all()?;
    drop(output);
    if worker.stopped() {
        return Ok(Vec::new());
    }
    if downloaded != total {
        return Err(io::Error::other("source changed during the copy"));
    }
    fs::rename(&partial, &result)?;
    Ok(vec!["copy.bin".into()])
}
