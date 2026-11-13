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
        System::Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject},
    };

    pub struct Instance {
        mutex: HANDLE,
        event: HANDLE,
        show_event: HANDLE,
        stopping: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        show_worker: Option<JoinHandle<()>>,
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    impl Instance {
        pub fn acquire(data_dir: &Path, show: bool) -> io::Result<Option<Self>> {
            let name = ndm_core::desktop_instance_namespace(data_dir);
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
                if exists {
                    let result = SetEvent(event);
                    let error = if result == 0 || (show && SetEvent(show_event) == 0) {
                        Some(io::Error::last_os_error())
                    } else {
                        None
                    };
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
                    stopping: Arc::new(AtomicBool::new(false)),
                    worker: None,
                    show_worker: None,
                }))
            }
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
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            if let Some(worker) = self.show_worker.take() {
                let _ = worker.join();
            }
            unsafe {
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
    }
}

#[cfg(windows)]
pub use platform::Instance;
