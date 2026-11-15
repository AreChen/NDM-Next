//! Per-profile macOS process coordination. File locks release automatically on exit.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{fs::OpenOptionsExt, net::UnixStream},
    path::Path,
    time::{Duration, Instant},
};

pub fn lock_file(data_dir: &Path, name: &str) -> io::Result<File> {
    std::fs::create_dir_all(data_dir)?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(data_dir.join(name))
}

pub fn request(data_dir: &Path, command: u8) -> io::Result<bool> {
    let mut stream = UnixStream::connect(data_dir.join("desktop.sock"))?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    stream.write_all(&[command])?;
    let mut response = [0];
    stream.read_exact(&mut response)?;
    Ok(response[0] == 1)
}

pub fn desktop_is_running(data_dir: &Path) -> io::Result<bool> {
    let file = lock_file(data_dir, "desktop.lock")?;
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

pub fn wait_for_desktop_ready(data_dir: &Path, timeout: Duration) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if request(data_dir, b'R').unwrap_or(false) {
            return Ok(true);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(25)));
    }
}

pub fn wake_desktop(data_dir: &Path) -> io::Result<()> {
    if request(data_dir, b'W')? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "desktop is not ready",
        ))
    }
}

pub fn request_desktop_shutdown(data_dir: &Path) -> io::Result<bool> {
    request(data_dir, b'Q')
}
