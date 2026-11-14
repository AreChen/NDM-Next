#[cfg(windows)]
mod platform {
    use std::{
        io,
        path::Path,
        ptr,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::Threading::{
            CreateEventW, CreateMutexW, INFINITE, ReleaseMutex, ResetEvent, SetEvent,
            WaitForSingleObject,
        },
    };

    pub struct Instance {
        mutex: HANDLE,
        event: HANDLE,
        show_event: HANDLE,
        ready_event: HANDLE,
        shutdown_event: HANDLE,
        shutdown_requested: Arc<AtomicBool>,
        stopping: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        show_worker: Option<JoinHandle<()>>,
        shutdown_worker: Option<JoinHandle<()>>,
    }

    struct StartupGuard(HANDLE);

    impl Drop for StartupGuard {
        fn drop(&mut self) {
            // SAFETY: Only a newly created, owned startup mutex reaches this type.
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    impl Instance {
        pub fn acquire(data_dir: &Path, show: bool) -> io::Result<Option<Self>> {
            let name = ndm_core::desktop_instance_namespace(data_dir);
            // Serialize the short registration step with installer guard acquisition.
            // Existing guards belong to an installer or another startup; never open storage.
            let startup = unsafe {
                let guard = CreateMutexW(
                    ptr::null(),
                    1,
                    wide(&ndm_core::desktop_installation_namespace(data_dir)).as_ptr(),
                );
                if guard.is_null() {
                    return Err(io::Error::last_os_error());
                }
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    CloseHandle(guard);
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "installation or startup is in progress",
                    ));
                }
                StartupGuard(guard)
            };
            let _startup = startup;
            // SAFETY: Both names are NUL-terminated, security attributes are
            // null (the current user's default DACL), and handles are closed below.
            unsafe {
                let mutex = CreateMutexW(ptr::null(), 0, wide(&format!("{name}-mutex")).as_ptr());
                if mutex.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let exists = GetLastError() == ERROR_ALREADY_EXISTS;
                let event = CreateEventW(ptr::null(), 0, 0, wide(&format!("{name}-wake")).as_ptr());
                if event.is_null() {
                    let error = io::Error::last_os_error();
                    CloseHandle(mutex);
                    return Err(error);
                }
                let show_event =
                    CreateEventW(ptr::null(), 0, 0, wide(&format!("{name}-show")).as_ptr());
                if show_event.is_null() {
                    let error = io::Error::last_os_error();
                    CloseHandle(event);
                    CloseHandle(mutex);
                    return Err(error);
                }
                let ready_event =
                    CreateEventW(ptr::null(), 1, 0, wide(&format!("{name}-ready")).as_ptr());
                if ready_event.is_null() {
                    let error = io::Error::last_os_error();
                    CloseHandle(show_event);
                    CloseHandle(event);
                    CloseHandle(mutex);
                    return Err(error);
                }
                let shutdown_event = CreateEventW(
                    ptr::null(),
                    0,
                    0,
                    wide(&format!("{name}-shutdown")).as_ptr(),
                );
                if shutdown_event.is_null() {
                    let error = io::Error::last_os_error();
                    CloseHandle(ready_event);
                    CloseHandle(show_event);
                    CloseHandle(event);
                    CloseHandle(mutex);
                    return Err(error);
                }
                if exists {
                    let result = SetEvent(event);
                    let error = if result == 0 || (show && SetEvent(show_event) == 0) {
                        Some(io::Error::last_os_error())
                    } else {
                        None
                    };
                    CloseHandle(ready_event);
                    CloseHandle(shutdown_event);
                    CloseHandle(show_event);
                    CloseHandle(event);
                    CloseHandle(mutex);
                    return match error {
                        Some(error) => Err(error),
                        None => Ok(None),
                    };
                }
                Ok(Some(Self {
                    mutex,
                    event,
                    show_event,
                    ready_event,
                    shutdown_event,
                    shutdown_requested: Arc::new(AtomicBool::new(false)),
                    stopping: Arc::new(AtomicBool::new(false)),
                    worker: None,
                    show_worker: None,
                    shutdown_worker: None,
                }))
            }
        }

        pub fn mark_ready(&self) -> io::Result<()> {
            // SAFETY: The instance owns the event until its Drop implementation.
            let result = if self.shutdown_requested.load(Ordering::Acquire) {
                unsafe { ResetEvent(self.ready_event) }
            } else {
                unsafe { SetEvent(self.ready_event) }
            };
            if result == 0 {
                Err(io::Error::last_os_error())
            } else {
                // A shutdown may race recovery's final SetEvent; it must remain unavailable.
                if self.shutdown_requested.load(Ordering::Acquire) {
                    unsafe {
                        ResetEvent(self.ready_event);
                    }
                }
                Ok(())
            }
        }

        pub fn on_shutdown(&mut self, notify: impl Fn() + Send + 'static) {
            let event = self.shutdown_event as usize;
            let ready = self.ready_event as usize;
            let stopping = self.stopping.clone();
            let requested = self.shutdown_requested.clone();
            self.shutdown_worker = Some(thread::spawn(move || {
                // SAFETY: Instance owns both handles until this worker has joined.
                if unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } == WAIT_OBJECT_0
                    && !stopping.load(Ordering::Acquire)
                {
                    requested.store(true, Ordering::Release);
                    unsafe {
                        ResetEvent(ready as HANDLE);
                    }
                    notify();
                }
            }));
        }

        pub fn on_wake(&mut self, notify: impl Fn() + Send + 'static) {
            let event = self.event as usize;
            let stopping = self.stopping.clone();
            self.worker = Some(thread::spawn(move || {
                loop {
                    // SAFETY: Instance owns this event until the worker has joined.
                    if unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } != WAIT_OBJECT_0 {
                        break;
                    }
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    notify();
                }
            }));
        }

        pub fn on_show(&mut self, notify: impl Fn() + Send + 'static) {
            let event = self.show_event as usize;
            let stopping = self.stopping.clone();
            self.show_worker = Some(thread::spawn(move || {
                loop {
                    // SAFETY: Instance keeps the event open until this worker exits.
                    if unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } != WAIT_OBJECT_0
                        || stopping.load(Ordering::Acquire)
                    {
                        break;
                    }
                    notify();
                }
            }));
        }
    }

    impl Drop for Instance {
        fn drop(&mut self) {
            self.stopping.store(true, Ordering::Release);
            // SAFETY: Handles were created by acquire and remain valid until closed.
            unsafe {
                SetEvent(self.event);
                SetEvent(self.show_event);
                SetEvent(self.shutdown_event);
                ResetEvent(self.ready_event);
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            if let Some(worker) = self.show_worker.take() {
                let _ = worker.join();
            }
            if let Some(worker) = self.shutdown_worker.take() {
                let _ = worker.join();
            }
            unsafe {
                CloseHandle(self.shutdown_event);
                CloseHandle(self.ready_event);
                CloseHandle(self.show_event);
                CloseHandle(self.event);
                CloseHandle(self.mutex);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{sync::mpsc, time::Duration};

        #[test]
        fn running_status_tracks_the_actual_instance() {
            let dir = tempfile::tempdir().unwrap();
            assert!(!ndm_core::desktop_is_running(dir.path()).unwrap());
            let first = Instance::acquire(dir.path(), false).unwrap().unwrap();
            assert!(ndm_core::desktop_is_running(dir.path()).unwrap());
            drop(first);
            assert!(!ndm_core::desktop_is_running(dir.path()).unwrap());
        }

        #[test]
        fn second_instance_wakes_first_and_does_not_hold_singleton() {
            let dir = tempfile::tempdir().unwrap();
            let mut first = Instance::acquire(dir.path(), true).unwrap().unwrap();
            let (send, receive) = mpsc::channel();
            first.on_wake(move || {
                let _ = send.send(());
            });
            let (show_send, show_receive) = mpsc::channel();
            first.on_show(move || {
                let _ = show_send.send(());
            });
            assert!(Instance::acquire(dir.path(), false).unwrap().is_none());
            receive.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(
                show_receive
                    .recv_timeout(Duration::from_millis(100))
                    .is_err()
            );
            assert!(Instance::acquire(dir.path(), true).unwrap().is_none());
            receive.recv_timeout(Duration::from_secs(2)).unwrap();
            show_receive.recv_timeout(Duration::from_secs(2)).unwrap();
            drop(first);
            assert!(Instance::acquire(dir.path(), true).unwrap().is_some());
        }

        #[test]
        fn ready_is_signaled_only_after_recovery_finishes() {
            let dir = tempfile::tempdir().unwrap();
            let first = Instance::acquire(dir.path(), false).unwrap().unwrap();
            assert!(
                !ndm_core::wait_for_desktop_ready(dir.path(), Duration::from_millis(25)).unwrap()
            );
            first.mark_ready().unwrap();
            assert!(ndm_core::wait_for_desktop_ready(dir.path(), Duration::from_secs(1)).unwrap());
        }

        #[test]
        fn installer_guard_rejects_new_desktop_instances() {
            let dir = tempfile::tempdir().unwrap();
            let name = format!(
                "{}-installing",
                ndm_core::desktop_instance_namespace(dir.path())
            );
            // SAFETY: This isolated profile owns the name and the handle is closed below.
            let guard = unsafe { CreateMutexW(ptr::null(), 1, wide(&name).as_ptr()) };
            assert!(!guard.is_null());
            let result = Instance::acquire(dir.path(), false);
            unsafe {
                CloseHandle(guard);
            }
            assert!(
                result.is_err(),
                "desktop startup must defer while files are replaced"
            );
        }

        #[test]
        fn shutdown_clears_readiness_before_notifying_and_recovery_cannot_restore_it() {
            let dir = tempfile::tempdir().unwrap();
            let mut instance = Instance::acquire(dir.path(), false).unwrap().unwrap();
            instance.mark_ready().unwrap();
            let profile = dir.path().to_owned();
            let (send, receive) = mpsc::channel();
            instance.on_shutdown(move || {
                send.send(ndm_core::wait_for_desktop_ready(&profile, Duration::ZERO).unwrap())
                    .unwrap();
            });
            assert!(ndm_core::request_desktop_shutdown(dir.path()).unwrap());
            assert!(!receive.recv_timeout(Duration::from_secs(2)).unwrap());
            instance.mark_ready().unwrap();
            assert!(!ndm_core::wait_for_desktop_ready(dir.path(), Duration::ZERO).unwrap());
        }
    }
}

#[cfg(windows)]
pub use platform::Instance;
