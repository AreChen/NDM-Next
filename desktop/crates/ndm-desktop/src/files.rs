use ndm_core::{Task, TaskState};
use std::{
    io,
    path::{Path, PathBuf},
};

fn completed_output(task: &Task) -> io::Result<PathBuf> {
    if task.state != TaskState::Completed {
        return Err(io::Error::other("download is not complete"));
    }
    let path = task
        .output_path
        .as_ref()
        .ok_or_else(|| io::Error::other("download has no output file"))?;
    let metadata = std::fs::symlink_metadata(path)?;
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
    std::path::absolute(path)
}

fn delete_output_with(task: &Task, remove: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    remove(&completed_output(task)?)
}

pub(crate) fn delete_output(task: &Task) -> io::Result<()> {
    delete_output_with(task, recycle)
}

pub(crate) fn open_file(task: &Task) -> io::Result<()> {
    open_path(&completed_output(task)?)
}

pub(crate) fn open_folder(task: &Task) -> io::Result<()> {
    let directory = std::path::absolute(&task.request.destination)?;
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

#[cfg(not(windows))]
fn open_path(_path: &Path) -> io::Result<()> {
    Err(io::Error::other(
        "opening files is only supported on Windows",
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

#[cfg(not(windows))]
fn recycle(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
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
}
