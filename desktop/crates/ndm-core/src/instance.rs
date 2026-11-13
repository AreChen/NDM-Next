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

/// Wait until the desktop has finished recovering old tasks and can accept new ones.
#[cfg(windows)]
pub fn wait_for_desktop_ready(
    data_dir: &Path,
    timeout: std::time::Duration,
) -> std::io::Result<bool> {
    use std::{thread, time::Instant};
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_FILE_NOT_FOUND, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        System::Threading::{OpenEventW, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject},
    };
    let name: Vec<u16> = format!("{}-ready", desktop_instance_namespace(data_dir))
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: The name is NUL-terminated; each opened handle is closed.
        let ready = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
        if !ready.is_null() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let millis = remaining.as_millis().min(u32::MAX as u128) as u32;
            let result = unsafe { WaitForSingleObject(ready, millis) };
            let error = if result != WAIT_OBJECT_0 && result != WAIT_TIMEOUT {
                Some(std::io::Error::last_os_error())
            } else {
                None
            };
            unsafe { CloseHandle(ready) };
            return match error {
                Some(error) => Err(error),
                None => Ok(result == WAIT_OBJECT_0),
            };
        }
        let error = unsafe { GetLastError() };
        if error != ERROR_FILE_NOT_FOUND {
            return Err(std::io::Error::from_raw_os_error(error as i32));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        thread::sleep(remaining.min(std::time::Duration::from_millis(25)));
    }
}

/// Wake the initialized desktop after a new task is durably committed.
#[cfg(windows)]
pub fn wake_desktop(data_dir: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent},
    };
    let name: Vec<u16> = format!("{}-wake", desktop_instance_namespace(data_dir))
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: The name is NUL-terminated, and an opened event is always closed.
    unsafe {
        let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if event.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let result = SetEvent(event);
        let error = if result == 0 {
            Some(std::io::Error::last_os_error())
        } else {
            None
        };
        CloseHandle(event);
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
