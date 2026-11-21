use std::{
    io,
    num::NonZeroU32,
    path::{Path, PathBuf},
};

use ndm_plugin_sdk::{
    DownloadRequest, DownloadSource, FailureReason, TaskUpdate, TransferProgress,
};
use tokio::fs;
use tokio_util::sync::CancellationToken;

use crate::{
    DownloadProgress, Error, PluginOptions, Result, Store, TaskId, cache,
    http::{publish_download, safe_filename},
    plugin_engine::{EngineError, EngineOutcome, EngineUpdate, PluginEngine},
    plugins::PluginRegistry,
};

pub(crate) async fn execute(
    store: &Store,
    id: TaskId,
    options: PluginOptions,
    default_threads: u32,
    cancel: CancellationToken,
    progress: DownloadProgress,
    title_changed: impl Fn(),
) -> Result<()> {
    store.start(id)?;
    let result = transfer(
        store,
        id,
        options,
        default_threads,
        &cancel,
        &progress,
        title_changed,
    )
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(Error::Cancelled) => store.pause(id),
        Err(error) => {
            store.fail(
                id,
                match error {
                    Error::PluginUnavailable => "download plugin is unavailable",
                    Error::PluginAuthentication => "download plugin authentication failed",
                    Error::PluginNetwork => "download plugin network error",
                    Error::PluginSource => "download plugin does not support this source",
                    Error::PluginStorage | Error::Io(_) => "download plugin storage error",
                    Error::PluginOptions => "plugin_options_invalid",
                    Error::PluginPostProcessing => "plugin_postprocessing_failed",
                    _ => "download plugin failed",
                },
            )?;
            Ok(())
        }
    }
}

async fn transfer(
    store: &Store,
    id: TaskId,
    options: PluginOptions,
    default_threads: u32,
    cancel: &CancellationToken,
    progress: &DownloadProgress,
    title_changed: impl Fn(),
) -> Result<()> {
    let task = store.get(id)?.ok_or(Error::NotFound)?;
    let plugin = PluginRegistry::new(store.profile_directory())
        .get(&options.id)?
        .ok_or(Error::PluginUnavailable)?;
    let source = reqwest::Url::parse(&task.request.url).map_err(|_| Error::PluginSource)?;
    let source = if source.scheme() == "file" {
        DownloadSource::File {
            path: source.to_file_path().map_err(|()| Error::PluginSource)?,
        }
    } else {
        DownloadSource::Url {
            url: source.to_string(),
            headers: task.request.headers.clone(),
        }
    };
    let workspace = cache::engine_workspace(&task)?;
    let engine = PluginEngine::new(plugin.executable())?;
    let download_threads = plugin.manifest.limits.download_threads.and_then(|limit| {
        NonZeroU32::new(
            task.policy
                .download_threads
                .unwrap_or(default_threads)
                .min(limit),
        )
    });
    let request = DownloadRequest {
        task_id: task.id,
        source,
        workspace: workspace.clone(),
        suggested_filename: task.request.suggested_filename.clone(),
        expected_size: task.request.expected_size,
        download_threads,
        options: options.values,
    };
    let mut resolved = false;
    let outcome = engine
        .download_with_updates(request, cancel, |update| match update {
            EngineUpdate::Progress(update) => {
                progress.resume_downloading();
                progress.engine_progress(update);
            }
            EngineUpdate::Task(TaskUpdate::Finalizing) => progress.finalize(),
            EngineUpdate::Task(TaskUpdate::Title { title }) if !resolved => {
                let title: String = title
                    .trim()
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(240)
                    .collect();
                if !title.is_empty() {
                    resolved = true;
                    if task.engine_title.as_deref() != Some(&title)
                        && store.set_engine_title(id, &title).unwrap_or(false)
                    {
                        title_changed();
                    }
                }
            }
            EngineUpdate::Task(TaskUpdate::Title { .. }) => {}
        })
        .await;
    let files = match outcome {
        Ok(EngineOutcome::Completed(files)) => files,
        Ok(EngineOutcome::Stopped) => return Err(Error::Cancelled),
        Err(_) if cancel.is_cancelled() => return Err(Error::Cancelled),
        Err(EngineError::Failed(FailureReason::Authentication)) => {
            return Err(Error::PluginAuthentication);
        }
        Err(EngineError::Failed(FailureReason::Network)) => return Err(Error::PluginNetwork),
        Err(EngineError::Failed(FailureReason::Storage)) => return Err(Error::PluginStorage),
        Err(EngineError::Failed(FailureReason::InvalidOptions)) => {
            return Err(Error::PluginOptions);
        }
        Err(EngineError::Failed(FailureReason::PostProcessing)) => {
            return Err(Error::PluginPostProcessing);
        }
        Err(EngineError::Failed(FailureReason::InvalidSource | FailureReason::Unsupported)) => {
            return Err(Error::PluginSource);
        }
        Err(_) => return Err(Error::PluginTransfer),
    };
    let destination = &task.request.destination;
    fs::create_dir_all(destination).await?;
    let mut bytes = 0u64;
    for file in &files {
        bytes = bytes.saturating_add(fs::metadata(file).await?.len());
    }
    if task
        .request
        .expected_size
        .is_some_and(|expected| bytes != expected)
    {
        return Err(Error::PluginTransfer);
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let output = if files.len() == 1 {
        let source = &files[0];
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("download");
        publish_download(source, destination, &safe_filename(name), progress, cancel).await?
    } else {
        progress.finalize();
        publish_group(
            &workspace,
            &files,
            destination,
            task.request.suggested_filename.as_deref(),
            cancel,
        )
        .await?
    };
    progress.engine_progress(TransferProgress {
        downloaded: bytes,
        total: Some(bytes),
        transferred: progress.snapshot().transferred,
    });
    store.complete(id, &output, task.request.expected_size.is_some())?;
    let root = task.cache_directory.as_deref().unwrap_or(destination);
    let _ = cache::remove_partial(root, id);
    Ok(())
}

async fn publish_group(
    workspace: &Path,
    files: &[PathBuf],
    destination: &Path,
    suggested: Option<&str>,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    let relative: Vec<&Path> = files
        .iter()
        .map(|file| {
            file.strip_prefix(workspace)
                .map_err(|_| Error::PluginTransfer)
        })
        .collect::<Result<_>>()?;
    let shared = relative[0]
        .components()
        .next()
        .map(|component| component.as_os_str());
    let shared = shared.filter(|prefix| {
        relative.iter().all(|path| {
            path.components().count() > 1
                && path
                    .components()
                    .next()
                    .is_some_and(|c| c.as_os_str() == *prefix)
        })
    });
    let name = safe_filename(
        shared
            .and_then(|name| name.to_str())
            .or(suggested)
            .unwrap_or("download"),
    );
    let stage = tempfile::Builder::new()
        .prefix(".ndm-next-publish-")
        .tempdir_in(destination)?;
    for (source, path) in files.iter().zip(relative) {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let path = shared
            .and_then(|prefix| path.strip_prefix(prefix).ok())
            .unwrap_or(path);
        let path = stage.path().join(path);
        let parent = path.parent().ok_or(Error::PluginTransfer)?;
        fs::create_dir_all(parent).await?;
        // Keep the engine's files until the whole folder has been published.
        if fs::hard_link(source, &path).await.is_err() {
            let mut input = fs::File::open(source).await?;
            let mut output = fs::File::from_std(std::fs::File::create_new(path)?);
            tokio::select! {
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                result = tokio::io::copy(&mut input, &mut output) => { result?; }
            }
            output.sync_all().await?;
        }
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    for index in 0..4096 {
        let name = if index == 0 {
            name.clone()
        } else {
            format!("{name} ({index})")
        };
        let target = destination.join(name);
        match rename_directory(stage.path(), &target) {
            Ok(()) => return Ok(target),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::PluginTransfer)
}

#[cfg(windows)]
fn rename_directory(from: &Path, to: &Path) -> io::Result<()> {
    // Windows directory rename refuses to replace an existing directory.
    std::fs::rename(from, to).map_err(|error| {
        if std::fs::symlink_metadata(to).is_ok() {
            io::Error::new(io::ErrorKind::AlreadyExists, error)
        } else {
            error
        }
    })
}

#[cfg(unix)]
fn rename_directory(from: &Path, to: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(from.as_os_str().as_bytes()).map_err(io::Error::other)?;
    let to = CString::new(to.as_os_str().as_bytes()).map_err(io::Error::other)?;
    // SAFETY: Both path pointers are NUL-terminated and live through the call.
    #[cfg(target_os = "linux")]
    let status = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    // SAFETY: Both path pointers are NUL-terminated and live through the call.
    #[cfg(target_os = "macos")]
    let status = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
