use ndm_core::{Task, TaskState};
use std::{
    io,
    path::{Path, PathBuf},
};

fn completed_output(task: &Task) -> io::Result<Option<PathBuf>> {
    if task.state != TaskState::Completed {
        return Err(io::Error::other("download is not complete"));
    }
    let Some(path) = task.output_path.as_ref() else {
        return Ok(None);
    };
    let metadata = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = metadata.file_type().is_symlink();
    if !metadata.is_file() || reparse {
        return Err(io::Error::other("download output is not a regular file"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("download has no parent directory"))?;
    if parent.canonicalize()? != task.request.destination.canonicalize()? {
        return Err(io::Error::other(
            "download output is outside its destination",
        ));
    }
    std::path::absolute(path).map(Some)
}

fn delete_output_with(task: &Task, remove: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    let Some(path) = completed_output(task)? else {
        return Ok(());
    };
    match remove(&path) {
        // Another process can remove the file after validation, or the shell
        // can report failure after removing it. Only an actual absence counts.
        Err(_) if matches!(std::fs::symlink_metadata(&path), Err(error) if error.kind() == io::ErrorKind::NotFound) => {
            Ok(())
        }
        result => result,
    }
}

pub(crate) fn delete_output(task: &Task) -> io::Result<()> {
    delete_output_with(task, recycle)
}

pub(crate) fn open_file(task: &Task) -> io::Result<()> {
    let path = completed_output(task)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "download output is missing"))?;
    open_path(&path)
}

pub(crate) fn open_folder(task: &Task) -> io::Result<()> {
    open_directory(&task.request.destination)
}

pub(crate) fn open_directory(path: &Path) -> io::Result<()> {
    let directory = std::path::absolute(path)?;
    if !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "download folder does not exist",
        ));
    }
    open_path(&directory)
}

#[cfg(windows)]
fn open_path(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the path is NUL-terminated and all optional pointers are null.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err(io::Error::other("Windows could not open the path"))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn open_path(path: &Path) -> io::Result<()> {
    if std::process::Command::new("xdg-open")
        .arg(path)
        .status()?
        .success()
    {
        Ok(())
    } else {
        Err(io::Error::other("Linux could not open the path"))
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn open_path(_path: &Path) -> io::Result<()> {
    Err(io::Error::other(
        "opening files is unavailable on this platform",
    ))
}

#[cfg(windows)]
fn recycle(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT, FOF_WANTNUKEWARNING,
        SHFILEOPSTRUCTW, SHFileOperationW,
    };
    let from: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut operation = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_WANTNUKEWARNING) as u16,
        ..Default::default()
    };
    // SAFETY: the structure and double-NUL-terminated source path remain valid for the call.
    let result = unsafe { SHFileOperationW(&mut operation) };
    if result != 0 || operation.fAnyOperationsAborted != 0 || path.exists() {
        Err(io::Error::other("Windows could not delete the download"))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn recycle(path: &Path) -> io::Result<()> {
    if std::process::Command::new("gio")
        .args(["trash", "--"])
        .arg(path)
        .status()?
        .success()
    {
        Ok(())
    } else {
        Err(io::Error::other(
            "Linux could not move the download to the trash",
        ))
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn recycle(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

#[cfg(target_os = "macos")]
fn open_path(path: &Path) -> io::Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg("--")
        .arg(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("macOS could not open the path"))
    }
}

#[cfg(target_os = "macos")]
fn recycle(path: &Path) -> io::Result<()> {
    use objc2_foundation::{NSFileManager, NSURL};
    let url = NSURL::from_file_path(path).ok_or_else(|| io::Error::other("invalid file URL"))?;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndm_core::{NewTask, TaskOptions};
    use std::collections::BTreeMap;

    fn task(destination: &std::path::Path, output: std::path::PathBuf, state: TaskState) -> Task {
        Task {
            cache_directory: None,
            id: ndm_core::TaskId::new_v4(),
            request: NewTask {
                url: "https://example.test/file".into(),
                destination: destination.into(),
                suggested_filename: None,
                source_url: None,
                expected_size: None,
                headers: BTreeMap::new(),
            },
            options: TaskOptions::Auto,
            state,
            output_path: Some(output),
            error: None,
            size_verified: Some(true),
            progress: None,
        }
    }

    #[test]
    fn only_completed_regular_output_in_destination_can_be_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("download.bin");
        std::fs::write(&output, b"download").unwrap();
        let mut item = task(directory.path(), output.clone(), TaskState::Paused);
        assert!(delete_output_with(&item, |path| std::fs::remove_file(path)).is_err());
        assert!(output.exists());
        item.state = TaskState::Completed;
        delete_output_with(&item, |path| std::fs::remove_file(path)).unwrap();
        assert!(!output.exists());
    }

    #[test]
    fn deletion_rejects_directories_and_files_outside_the_destination() {
        let destination = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("keep.bin");
        std::fs::write(&file, b"keep").unwrap();
        assert!(
            delete_output_with(
                &task(destination.path(), file.clone(), TaskState::Completed),
                |path| std::fs::remove_file(path)
            )
            .is_err()
        );
        assert!(file.exists());
        assert!(
            delete_output_with(
                &task(
                    destination.path(),
                    destination.path().into(),
                    TaskState::Completed
                ),
                |path| std::fs::remove_file(path)
            )
            .is_err()
        );
        assert!(destination.path().is_dir());
    }

    #[test]
    fn missing_outputs_are_idempotent_but_real_file_errors_are_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("missing.bin");
        let mut item = task(directory.path(), output.clone(), TaskState::Completed);
        delete_output_with(&item, |_| {
            panic!("Missing file must not reach the recycler")
        })
        .unwrap();
        item.output_path = None;
        delete_output_with(&item, |_| panic!("No file to recycle")).unwrap();
        item.output_path = Some(output.clone());
        std::fs::write(&output, b"keep").unwrap();
        assert!(
            delete_output_with(&item, |_| Err(io::Error::from(
                io::ErrorKind::PermissionDenied
            )))
            .is_err()
        );
        assert!(output.exists());
        delete_output_with(&item, |path| {
            std::fs::remove_file(path)?;
            Err(io::Error::other("shell reported failure after removal"))
        })
        .unwrap();
    }
}
