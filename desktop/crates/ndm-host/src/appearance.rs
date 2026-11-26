//! Read-only appearance updates, scoped to the running desktop profile.
use std::{
    io::{self, Read},
    path::Path,
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    runtime::Runtime,
    sync::{Semaphore, watch},
    task::JoinHandle,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub mode: Mode,
    pub primary: u32,
    pub secondary: u32,
    pub surface: u32,
}

pub struct Publisher {
    value: watch::Sender<Appearance>,
    listener: JoinHandle<()>,
}

impl Publisher {
    /// The desktop holds its profile's singleton lock while publishing.
    pub fn start(profile: &Path, runtime: &Runtime, initial: Appearance) -> io::Result<Self> {
        let _runtime = runtime.enter();
        let mut listener = local::Listener::bind(profile)?;
        let (value, receiver) = watch::channel(initial);
        let listener = runtime.spawn(async move {
            let slots = Arc::new(Semaphore::new(16));
            while let Ok(stream) = listener.accept().await {
                let Ok(slot) = slots.clone().try_acquire_owned() else {
                    continue;
                };
                let receiver = receiver.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    let _ = serve(stream, receiver).await;
                });
            }
        });
        Ok(Self { value, listener })
    }

    pub fn update(&self, appearance: Appearance) {
        self.value.send_if_modified(|value| {
            if *value == appearance {
                return false;
            }
            *value = appearance;
            true
        });
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

async fn serve(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    mut value: watch::Receiver<Appearance>,
) -> io::Result<()> {
    loop {
        let appearance = *value.borrow_and_update();
        let payload = serde_json::to_vec(&appearance)?;
        stream.write_u32_le(payload.len() as u32).await?;
        stream.write_all(&payload).await?;
        tokio::select! {
            // EOF releases the subscription. The desktop accepts no commands here.
            _ = stream.read_u8() => return Ok(()),
            changed = value.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

pub fn connect(profile: &Path) -> io::Result<impl Read + Send + 'static> {
    local::connect(profile)
}

pub fn read(stream: &mut impl Read) -> io::Result<Option<Appearance>> {
    let Some(payload) = crate::read_frame(stream)? else {
        return Ok(None);
    };
    let value: Appearance = serde_json::from_slice(&payload)?;
    if [value.primary, value.secondary, value.surface]
        .into_iter()
        .any(|rgb| rgb > 0xffffff)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid appearance color",
        ));
    }
    Ok(Some(value))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod local {
    use std::{
        fs, io,
        os::unix::{fs::PermissionsExt, net::UnixStream},
        path::{Path, PathBuf},
    };
    use tokio::net::UnixListener;

    fn path(profile: &Path) -> io::Result<PathBuf> {
        Ok(ndm_core::unix_instance::socket_path(profile)?.with_extension("ui"))
    }

    pub(super) struct Listener(UnixListener, PathBuf);

    impl Listener {
        pub(super) fn bind(profile: &Path) -> io::Result<Self> {
            let path = path(profile)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let listener = Self(UnixListener::bind(&path)?, path);
            fs::set_permissions(&listener.1, fs::Permissions::from_mode(0o600))?;
            Ok(listener)
        }

        pub(super) async fn accept(&mut self) -> io::Result<tokio::net::UnixStream> {
            self.0.accept().await.map(|(stream, _)| stream)
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.1);
        }
    }

    pub(super) fn connect(profile: &Path) -> io::Result<UnixStream> {
        UnixStream::connect(path(profile)?)
    }
}

#[cfg(windows)]
mod local {
    use std::{
        fs::{File, OpenOptions},
        io,
        path::Path,
    };
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use windows_sys::Win32::{Foundation::ERROR_PIPE_BUSY, System::Pipes::WaitNamedPipeW};

    fn path(profile: &Path) -> String {
        let namespace = ndm_core::desktop_instance_namespace(profile).replace('\\', "-");
        format!(r"\\.\pipe\{namespace}-appearance")
    }

    pub(super) struct Listener(NamedPipeServer, String);

    fn server(path: &str, first: bool) -> io::Result<NamedPipeServer> {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .in_buffer_size(256)
            .out_buffer_size(256)
            .create(path)
    }

    impl Listener {
        pub(super) fn bind(profile: &Path) -> io::Result<Self> {
            let path = path(profile);
            Ok(Self(server(&path, true)?, path))
        }

        pub(super) async fn accept(&mut self) -> io::Result<NamedPipeServer> {
            self.0.connect().await?;
            // Make the next instance available before serving this connection.
            let next = server(&self.1, false)?;
            Ok(std::mem::replace(&mut self.0, next))
        }
    }

    pub(super) fn connect(profile: &Path) -> io::Result<File> {
        let path = path(profile);
        let open = || OpenOptions::new().read(true).write(true).open(&path);
        match open() {
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                let name: Vec<_> = path.encode_utf16().chain(Some(0)).collect();
                // SAFETY: the pipe name is null-terminated and stays alive during the wait.
                if unsafe { WaitNamedPipeW(name.as_ptr(), 500) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                open()
            }
            result => result,
        }
    }
}
