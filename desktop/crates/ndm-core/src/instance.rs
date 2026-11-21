use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
};

/// Lifetime reservation for an idle desktop update. Dropping it reopens admission.
pub struct DesktopUpdateGuard {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    _file: std::fs::File,
    #[cfg(windows)]
    _marker: std::os::windows::io::OwnedHandle,
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    namespace: String,
}

#[cfg(target_os = "macos")]
impl DesktopUpdateGuard {
    /// Keep the update reservation locked across the desktop's exit.
    pub fn into_installer_stdin(self) -> std::process::Stdio {
        self._file.into()
    }
}

// Unlike the existing singleton protocol, these new names normalize aliases so
// Store handles opening the same profile share one admission boundary. Keep the
// legacy singleton/shutdown/installer namespace algorithm unchanged.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn coordination_namespace(data_dir: &Path) -> String {
    let path = std::fs::canonicalize(data_dir)
        .or_else(|_| std::path::absolute(data_dir))
        .unwrap_or_else(|_| data_dir.to_owned());
    let text = path.to_string_lossy();
    #[cfg(windows)]
    let text = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    };
    desktop_instance_namespace(Path::new(&*text))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn update_namespace(data_dir: &Path) -> String {
    format!("{}-updating", coordination_namespace(data_dir))
}

/// Register a desktop under the same boundary as work/update admission. The
/// closure is synchronous; no thread-owned admission handle escapes this call.
pub fn with_desktop_startup<T>(
    data_dir: &Path,
    startup: impl FnOnce() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let _admission = WorkAdmission::acquire(data_dir)?;
    if desktop_installation_active(data_dir)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "desktop update is in progress",
        ));
    }
    startup()
}

pub(crate) struct WorkAdmission {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    _file: std::fs::File,
    #[cfg(windows)]
    handle: std::os::windows::io::OwnedHandle,
    #[cfg(windows)]
    _same_thread: std::marker::PhantomData<*mut ()>,
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl WorkAdmission {
    pub(crate) fn acquire(data_dir: &Path) -> std::io::Result<Self> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let file = crate::unix_instance::lock_file(data_dir, "admission.lock")?;
            file.lock()?;
            Ok(Self { _file: file })
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::FromRawHandle;
            use windows_sys::Win32::{
                Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT},
                System::Threading::{CreateMutexW, WaitForSingleObject},
            };
            let name: Vec<_> = format!("{}-admission", coordination_namespace(data_dir))
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // SAFETY: The name is terminated; the owned handle closes on all paths.
            let raw = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            if raw.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let handle = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) };
            match unsafe { WaitForSingleObject(raw, 5_000) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self {
                    handle,
                    _same_thread: std::marker::PhantomData,
                }),
                WAIT_TIMEOUT => Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "work admission is busy",
                )),
                _ => Err(std::io::Error::last_os_error()),
            }
        }
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        {
            let _ = data_dir;
            Ok(Self {
                _lock: PORTABLE_ADMISSION
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
            })
        }
    }
}

#[cfg(windows)]
impl Drop for WorkAdmission {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: WorkAdmission cannot move across threads and acquired this mutex.
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.handle.as_raw_handle());
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
static PORTABLE_ADMISSION: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
static PORTABLE_RESERVATIONS: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
impl Drop for DesktopUpdateGuard {
    fn drop(&mut self) {
        PORTABLE_RESERVATIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.namespace);
    }
}

/// Called only while the profile admission mutex is held.
pub(crate) fn publish_update_reservation(
    data_dir: &Path,
) -> std::io::Result<Option<DesktopUpdateGuard>> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let file = crate::unix_instance::lock_file(data_dir, "updating.lock")?;
        match file.try_lock() {
            Ok(()) => Ok(Some(DesktopUpdateGuard { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let namespace = update_namespace(data_dir);
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::{
            Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
            System::Threading::CreateMutexW,
        };
        let name: Vec<_> = namespace.encode_utf16().chain(Some(0)).collect();
        // This is an existence marker, not an owned mutex: its Send guard may be
        // dropped on another Tokio worker. Kernel handle lifetime clears crashes.
        let raw = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let marker = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) };
        Ok((!existed).then_some(DesktopUpdateGuard { _marker: marker }))
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let mut reservations = PORTABLE_RESERVATIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if reservations.insert(namespace.clone()) {
            Ok(Some(DesktopUpdateGuard { namespace }))
        } else {
            Ok(None)
        }
    }
}

/// Shared namespace for the desktop singleton and its installer integration.
pub fn desktop_instance_namespace(data_dir: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    data_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("Global\\NDMNext-{:016x}", hasher.finish())
}

/// Per-profile replacement guard shared by the installer and application startup.
pub fn desktop_installation_namespace(data_dir: &Path) -> String {
    format!("{}-installing", desktop_instance_namespace(data_dir))
}

/// Check without creating the installation guard or taking ownership of it.
#[cfg(windows)]
pub fn desktop_installation_active(data_dir: &Path) -> std::io::Result<bool> {
    Ok(
        named_mutex_exists(&desktop_installation_namespace(data_dir))?
            || named_mutex_exists(&format!("{}-installing", coordination_namespace(data_dir)))?
            || named_mutex_exists(&update_namespace(data_dir))?,
    )
}

/// The installer's guard, excluding the desktop's own update reservation.
#[cfg(windows)]
pub fn windows_installer_active(data_dir: &Path) -> std::io::Result<bool> {
    named_mutex_exists(&desktop_installation_namespace(data_dir))
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn desktop_installation_active(data_dir: &Path) -> std::io::Result<bool> {
    Ok(PORTABLE_RESERVATIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(&update_namespace(data_dir)))
}

/// Check without creating or signalling the singleton object.
#[cfg(windows)]
pub fn desktop_is_running(data_dir: &Path) -> std::io::Result<bool> {
    named_mutex_exists(&format!("{}-mutex", desktop_instance_namespace(data_dir)))
}

#[cfg(windows)]
fn named_mutex_exists(name: &str) -> std::io::Result<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, GetLastError},
        System::Threading::{MUTEX_MODIFY_STATE, OpenMutexW},
    };
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
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

/// Request the desktop's regular graceful Quit path. False means an older desktop
/// has no shutdown event; callers must keep any legacy fallback scoped to its image.
#[cfg(windows)]
pub fn request_desktop_shutdown(data_dir: &Path) -> std::io::Result<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, GetLastError},
        System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent},
    };
    let name: Vec<u16> = format!("{}-shutdown", desktop_instance_namespace(data_dir))
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: The name is terminated and every successfully opened handle is closed.
    unsafe {
        let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if event.is_null() {
            return match GetLastError() {
                ERROR_FILE_NOT_FOUND => Ok(false),
                error => Err(std::io::Error::from_raw_os_error(error as i32)),
            };
        }
        let result = SetEvent(event);
        let error = if result == 0 {
            Some(std::io::Error::last_os_error())
        } else {
            None
        };
        CloseHandle(event);
        error.map_or(Ok(true), Err)
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn desktop_installation_active(data_dir: &Path) -> std::io::Result<bool> {
    let file = crate::unix_instance::lock_file(data_dir, "updating.lock")?;
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use crate::unix_instance::{
    desktop_is_running, request_desktop_shutdown, wait_for_desktop_ready, wake_desktop,
};
