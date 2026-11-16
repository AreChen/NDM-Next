//! Windows installer shutdown. Image paths and ancestry, never executable names,
//! determine which processes this installation owns.

use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet, HashMap},
    fs, io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
            QueryFullProcessImageNameW, SYNCHRONIZATION_SYNCHRONIZE, TerminateProcess,
            WaitForSingleObject,
        },
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle owns a valid, non-inherited Windows handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Process {
    pid: u32,
    image: String,
    handle: Handle,
    termination_requested: Cell<bool>,
}
impl Process {
    fn exited(&self) -> io::Result<bool> {
        // SAFETY: The process handle remains owned until this Process is dropped.
        match unsafe { WaitForSingleObject(self.handle.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }
    fn terminate(&self) -> io::Result<()> {
        if self.termination_requested.get() || self.exited()? {
            return Ok(());
        }
        // Keep the original process handle alive, preventing its PID from being
        // reused, and recheck the full image before requesting termination rights.
        let handle = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                self.pid,
            )
        };
        if handle.is_null() {
            return if self.exited()? {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            };
        }
        let handle = Handle(handle);
        if !image_path(handle.0).is_some_and(|path| same_image(&path, &self.image)) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "process image changed during shutdown",
            ));
        }
        // SAFETY: This handle was opened for the exact retained process and image.
        if unsafe { TerminateProcess(handle.0, 0) } == 0 && !self.exited()? {
            return Err(io::Error::last_os_error());
        }
        // TerminateProcess is asynchronous. Later shutdown passes retain this
        // handle and wait for exit rather than querying or terminating it again
        // while Windows is tearing down the process image.
        self.termination_requested.set(true);
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Desktop,
    Host,
    Media,
}
struct Selected {
    process: Process,
    kind: Kind,
}
struct Snapshot {
    parents: HashMap<u32, u32>,
    processes: Vec<Process>,
}

/// Gracefully stop this profile's desktop, then release installation-owned hosts
/// and media children. The caller must hold `desktop_installation_namespace`
/// throughout shutdown and replacement to prevent a new desktop or host startup.
///
/// An older desktop without the shutdown event permits termination of its exact
/// canonical image only. An existing event always gets the graceful deadline;
/// expiration fails the installation and never force-stops active desktop jobs.
pub fn shutdown_desktop_installation(
    data_dir: &Path,
    expected_desktop: &Path,
    timeout: Duration,
) -> io::Result<()> {
    if !expected_desktop.is_absolute()
        || !expected_desktop
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ndm-desktop.exe"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected an absolute desktop installation image",
        ));
    }
    let desktop = canonical_image(expected_desktop)?;
    let directory = expected_desktop.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing installation directory",
        )
    })?;
    let host = canonical_image(&directory.join("ndm-host.exe"))?;
    let media = [
        canonical_image(&directory.join("ffmpeg/ffmpeg.exe"))?,
        canonical_image(&directory.join("ffmpeg/ffprobe.exe"))?,
    ];
    let mut selected = BTreeMap::new();
    collect(snapshot()?, &mut selected, &desktop, &host, &media);
    let has_desktop = selected.values().any(|item| item.kind == Kind::Desktop);
    if !has_desktop && crate::desktop_is_running(data_dir)? {
        // A different installation owns the profile. Do not signal its event.
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "another installation owns this desktop profile",
        ));
    }
    if has_desktop {
        if crate::request_desktop_shutdown(data_dir)? {
            let deadline = Instant::now() + timeout;
            loop {
                collect(snapshot()?, &mut selected, &desktop, &host, &media);
                let mut running = false;
                for item in selected.values().filter(|item| item.kind == Kind::Desktop) {
                    running |= !item.process.exited()?;
                }
                if !running {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "desktop did not finish graceful shutdown; installation cancelled",
                    ));
                }
                thread::sleep(Duration::from_millis(25));
            }
        } else {
            for item in selected.values().filter(|item| item.kind == Kind::Desktop) {
                item.process.terminate()?;
            }
            wait_for_exit(&selected, Some(Kind::Desktop), Duration::from_secs(5))?;
        }
    }
    // Retained desktop/host IDs keep ancestry attributable after their exit.
    // Stdin-blocked native hosts cannot receive the desktop event; only exact
    // installation host images are stopped. Independent media tools survive.
    collect(snapshot()?, &mut selected, &desktop, &host, &media);
    for item in selected.values().filter(|item| item.kind == Kind::Host) {
        item.process.terminate()?;
    }
    collect(snapshot()?, &mut selected, &desktop, &host, &media);
    for item in selected.values().filter(|item| item.kind != Kind::Desktop) {
        item.process.terminate()?;
    }
    wait_for_exit(&selected, None, Duration::from_secs(5))
}

fn wait_for_exit(
    selected: &BTreeMap<u32, Selected>,
    kind: Option<Kind>,
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut running = false;
        for item in selected
            .values()
            .filter(|item| kind.is_none_or(|kind| item.kind == kind))
        {
            running |= !item.process.exited()?;
        }
        if !running {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "installation processes have not exited",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn collect(
    snapshot: Snapshot,
    selected: &mut BTreeMap<u32, Selected>,
    desktop: &str,
    host: &str,
    media: &[String; 2],
) {
    let mut ancestors: BTreeSet<u32> = selected
        .iter()
        .filter(|(_, item)| item.kind != Kind::Media)
        .map(|(pid, _)| *pid)
        .collect();
    for process in &snapshot.processes {
        if same_image(&process.image, desktop) || same_image(&process.image, host) {
            ancestors.insert(process.pid);
        }
    }
    for process in snapshot.processes {
        let kind = if same_image(&process.image, desktop) {
            Some(Kind::Desktop)
        } else if same_image(&process.image, host) {
            Some(Kind::Host)
        } else if media.iter().any(|path| same_image(&process.image, path))
            && descendant(process.pid, &snapshot.parents, &ancestors)
        {
            Some(Kind::Media)
        } else {
            None
        };
        if let Some(kind) = kind {
            selected
                .entry(process.pid)
                .or_insert(Selected { process, kind });
        }
    }
}

fn descendant(mut pid: u32, parents: &HashMap<u32, u32>, ancestors: &BTreeSet<u32>) -> bool {
    let mut seen = BTreeSet::new();
    while let Some(parent) = parents.get(&pid) {
        if !seen.insert(*parent) {
            return false;
        }
        if ancestors.contains(parent) {
            return true;
        }
        pid = *parent;
    }
    false
}

fn snapshot() -> io::Result<Snapshot> {
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let handle = Handle(handle);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut parents = HashMap::new();
    let mut processes = Vec::new();
    let mut present = unsafe { Process32FirstW(handle.0, &mut entry) };
    while present != 0 {
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        let length = entry
            .szExeFile
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExeFile.len());
        // Name is only a cheap candidate filter. Selection always verifies the
        // complete canonical image, and media selection also verifies ancestry.
        let candidate = String::from_utf16(&entry.szExeFile[..length]).is_ok_and(|name| {
            [
                "ndm-desktop.exe",
                "ndm-host.exe",
                "ffmpeg.exe",
                "ffprobe.exe",
            ]
            .iter()
            .any(|expected| name.eq_ignore_ascii_case(expected))
        });
        if candidate && entry.th32ProcessID != std::process::id() {
            let process = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZATION_SYNCHRONIZE,
                    0,
                    entry.th32ProcessID,
                )
            };
            if !process.is_null() {
                let process = Handle(process);
                if let Some(image) = image_path(process.0) {
                    processes.push(Process {
                        pid: entry.th32ProcessID,
                        image,
                        handle: process,
                        termination_requested: Cell::new(false),
                    });
                }
            }
        }
        present = unsafe { Process32NextW(handle.0, &mut entry) };
    }
    let error = unsafe { GetLastError() };
    if error != ERROR_NO_MORE_FILES {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(Snapshot { parents, processes })
}

fn image_path(handle: HANDLE) -> Option<String> {
    let mut image = vec![0_u16; 32_768];
    let mut length = image.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, image.as_mut_ptr(), &mut length) } == 0 {
        return None;
    }
    let image = String::from_utf16(&image[..length as usize]).ok()?;
    canonical_image(Path::new(&image)).ok()
}

fn canonical_image(path: &Path) -> io::Result<String> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "installation image must be absolute",
        ));
    }
    let mut ancestor = path.to_owned();
    let mut missing = Vec::new();
    let mut canonical: PathBuf;
    loop {
        match fs::canonicalize(&ancestor) {
            Ok(path) => {
                canonical = path;
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "invalid installation image",
                            )
                        })?
                        .to_owned(),
                );
                if !ancestor.pop() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    canonical.into_os_string().into_string().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid installation image encoding",
        )
    })
}

fn same_image(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}
