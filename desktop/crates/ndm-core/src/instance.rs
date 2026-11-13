use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
};

/// Shared namespace for the desktop singleton and its installer integration.
pub fn desktop_instance_namespace(data_dir: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    data_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("Global\\NDMNext-{:016x}", hasher.finish())
}

/// Check without creating or signalling the singleton object.
#[cfg(windows)]
pub fn desktop_is_running(data_dir: &Path) -> std::io::Result<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, GetLastError},
        System::Threading::{MUTEX_MODIFY_STATE, OpenMutexW},
    };
    let name: Vec<u16> = format!("{}-mutex", desktop_instance_namespace(data_dir))
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: The name is NUL-terminated and the returned handle is closed.
    unsafe {
        let mutex = OpenMutexW(MUTEX_MODIFY_STATE, 0, name.as_ptr());
        if mutex.is_null() {
            return match GetLastError() {
                ERROR_FILE_NOT_FOUND => Ok(false),
                error => Err(std::io::Error::from_raw_os_error(error as i32)),
            };
        }
        CloseHandle(mutex);
        Ok(true)
    }
}
