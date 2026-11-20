use std::{
    fs::File,
    io::{self, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

type Callback = Option<Box<dyn Fn() + Send>>;
#[derive(Default)]
struct Callbacks {
    wake: Callback,
    show: Callback,
    shutdown: Callback,
}

pub struct Instance {
    _lock: File,
    socket: PathBuf,
    ready: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    callbacks: Arc<Mutex<Callbacks>>,
    worker: Option<JoinHandle<()>>,
}

impl Instance {
    pub fn acquire(data_dir: &Path, show_existing: bool) -> io::Result<Option<Self>> {
        let instance = ndm_core::with_desktop_startup(data_dir, || {
            let lock = ndm_core::unix_instance::lock_file(data_dir, "desktop.lock")?;
            match lock.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
                Err(std::fs::TryLockError::Error(error)) => return Err(error),
            }
            let socket = ndm_core::unix_instance::socket_path(data_dir)?;
            match std::fs::remove_file(&socket) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let listener = UnixListener::bind(&socket)?;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            let ready = Arc::new(AtomicBool::new(false));
            let stopping = Arc::new(AtomicBool::new(false));
            let callbacks = Arc::new(Mutex::new(Callbacks::default()));
            let worker = {
                let ready = ready.clone();
                let stopping = stopping.clone();
                let callbacks = callbacks.clone();
                thread::spawn(move || {
                    while !stopping.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((mut stream, _)) => {
                                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                                let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                                let mut command = [0];
                                if stream.read_exact(&mut command).is_err() {
                                    continue;
                                }
                                let mut accepted = ready.load(Ordering::Acquire);
                                if accepted {
                                    let callbacks = callbacks.lock().unwrap();
                                    match command[0] {
                                        b'R' => {}
                                        b'W' => {
                                            if let Some(callback) = &callbacks.wake {
                                                callback();
                                            }
                                        }
                                        b'S' => {
                                            if let Some(callback) = &callbacks.wake {
                                                callback();
                                            }
                                            if let Some(callback) = &callbacks.show {
                                                callback();
                                            }
                                        }
                                        b'Q' => {
                                            ready.store(false, Ordering::Release);
                                            if let Some(callback) = &callbacks.shutdown {
                                                callback();
                                            }
                                        }
                                        _ => accepted = false,
                                    }
                                }
                                let _ = stream.write_all(&[u8::from(accepted)]);
                            }
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(25))
                            }
                            Err(_) => break,
                        }
                    }
                })
            };
            Ok(Some(Self {
                _lock: lock,
                socket,
                ready,
                stopping,
                callbacks,
                worker: Some(worker),
            }))
        })?;
        if instance.is_none() {
            // The first process needs admission to restore its task queue before
            // it can mark itself ready or accept a wake request.
            let _ = ndm_core::wait_for_desktop_ready(data_dir, Duration::from_secs(20));
            ndm_core::unix_instance::request(data_dir, if show_existing { b'S' } else { b'W' })?;
        }
        Ok(instance)
    }

    pub fn mark_ready(&self) -> io::Result<()> {
        self.ready.store(true, Ordering::Release);
        Ok(())
    }
    pub fn on_wake(&mut self, callback: impl Fn() + Send + 'static) {
        self.callbacks.lock().unwrap().wake = Some(Box::new(callback));
    }
    pub fn on_show(&mut self, callback: impl Fn() + Send + 'static) {
        self.callbacks.lock().unwrap().show = Some(Box::new(callback));
    }
    pub fn on_shutdown(&mut self, callback: impl Fn() + Send + 'static) {
        self.callbacks.lock().unwrap().shutdown = Some(Box::new(callback));
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.ready.store(false, Ordering::Release);
        self.stopping.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn singleton_readiness_wake_and_shutdown_follow_process_lifetime() {
        let dir = tempfile::tempdir().unwrap();
        let mut instance = Instance::acquire(dir.path(), false).unwrap().unwrap();
        assert!(ndm_core::desktop_is_running(dir.path()).unwrap());
        assert!(!ndm_core::wait_for_desktop_ready(dir.path(), Duration::ZERO).unwrap());
        let (send, receive) = std::sync::mpsc::channel();
        instance.on_wake(move || {
            send.send("wake").unwrap();
        });
        let (send, shutdown) = std::sync::mpsc::channel();
        instance.on_shutdown(move || {
            send.send("quit").unwrap();
        });
        instance.mark_ready().unwrap();
        assert!(ndm_core::wait_for_desktop_ready(dir.path(), Duration::from_secs(1)).unwrap());
        assert!(Instance::acquire(dir.path(), false).unwrap().is_none());
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(1)).unwrap(),
            "wake"
        );
        assert!(ndm_core::request_desktop_shutdown(dir.path()).unwrap());
        assert_eq!(
            shutdown.recv_timeout(Duration::from_secs(1)).unwrap(),
            "quit"
        );
        assert!(!ndm_core::wait_for_desktop_ready(dir.path(), Duration::ZERO).unwrap());
        drop(instance);
        assert!(!ndm_core::desktop_is_running(dir.path()).unwrap());
        assert!(Instance::acquire(dir.path(), true).unwrap().is_some());
    }
}
