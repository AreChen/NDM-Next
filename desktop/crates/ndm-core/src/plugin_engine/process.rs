use std::io;

use tokio::process::{Child, Command};

pub(super) struct Worker {
    pub(super) child: Child,
    tree: Option<ProcessTree>,
}

impl Worker {
    pub(super) fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        let child = command.spawn()?;
        let tree = ProcessTree::attach(&child)?;
        Ok(Self {
            child,
            tree: Some(tree),
        })
    }

    fn terminate(&mut self) -> io::Result<()> {
        if let Some(tree) = self.tree.take() {
            tree.terminate()?;
        }
        Ok(())
    }

    pub(super) async fn reap(mut self) -> io::Result<()> {
        let terminated = self.terminate();
        // macOS can return EPERM while the last process in a group is exiting.
        // Confirm that the worker exits, with the same deadline as a stop request.
        #[cfg(target_os = "macos")]
        if terminated
            .as_ref()
            .is_err_and(|error| error.raw_os_error() == Some(libc::EPERM))
            && let Ok(status) = tokio::time::timeout(super::STOP_TIMEOUT, self.child.wait()).await
        {
            return status.map(|_| ());
        }
        if terminated.is_err() {
            self.child.start_kill()?;
        }
        self.child.wait().await?;
        terminated
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(unix)]
struct ProcessTree(i32);

#[cfg(unix)]
impl ProcessTree {
    fn attach(child: &Child) -> io::Result<Self> {
        let pid = child.id().and_then(|pid| i32::try_from(pid).ok());
        pid.map(Self)
            .ok_or_else(|| io::Error::other("download engine has no process group"))
    }

    fn terminate(self) -> io::Result<()> {
        // SAFETY: spawn placed the engine in its own group. The group leader
        // stays owned by Child and has not been reaped when this is called.
        if unsafe { libc::kill(-self.0, libc::SIGKILL) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(windows)]
struct ProcessTree(std::os::windows::io::OwnedHandle);

#[cfg(windows)]
impl ProcessTree {
    fn attach(child: &Child) -> io::Result<Self> {
        use std::{
            mem::size_of,
            os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
            ptr,
        };

        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // SAFETY: null attributes and name create an unnamed, non-inherited job.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: this process owns the handle returned by CreateJobObjectW.
        let job = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("download engine has no process handle"))?;
        // SAFETY: both handles stay live and the limit structure matches its class.
        let configured = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0
                && AssignProcessToJobObject(job.as_raw_handle(), process) != 0
        };
        if configured {
            Ok(Self(job))
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn terminate(self) -> io::Result<()> {
        drop(self.0);
        Ok(())
    }
}
