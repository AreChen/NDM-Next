//! Optional registration adapters for local AI clients.
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::Deserialize;

mod skill;

const SERVER: &str = "ndm-next";

pub struct Target {
    pub executable: PathBuf,
    pub profile: PathBuf,
    pub home: PathBuf,
}

impl Target {
    pub fn for_desktop(profile: PathBuf) -> io::Result<Self> {
        let executable = std::env::current_exe()?.with_file_name(if cfg!(windows) {
            "ndm-host.exe"
        } else {
            "ndm-host"
        });
        let home =
            std::env::home_dir().ok_or_else(|| io::Error::other("user home is unavailable"))?;
        Ok(Self {
            executable,
            profile,
            home,
        })
    }

    fn skill_path(&self) -> PathBuf {
        self.home
            .join(".agents")
            .join("skills")
            .join(SERVER)
            .join("SKILL.md")
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Status {
    pub registered: bool,
    pub skill_installed: bool,
}

pub trait Harness {
    fn status(&self, target: &Target) -> io::Result<Status>;
    fn install(&self, target: &Target) -> io::Result<()>;
    fn remove(&self, target: &Target) -> io::Result<()>;
}

pub struct CodexCli;

#[derive(Deserialize)]
struct Server {
    name: String,
    enabled: bool,
    transport: Transport,
}

#[derive(Deserialize)]
struct Transport {
    #[serde(rename = "type")]
    kind: String,
    command: Option<PathBuf>,
    #[serde(default)]
    args: Vec<String>,
}

impl Transport {
    fn managed(&self, target: &Target) -> bool {
        self.kind == "stdio"
            && self
                .command
                .as_deref()
                .and_then(Path::file_name)
                .is_some_and(|name| {
                    name == if cfg!(windows) {
                        "ndm-host.exe"
                    } else {
                        "ndm-host"
                    }
                })
            && matches!(&self.args[..], [mode, flag, profile] if mode == "--mcp"
                && flag == "--data-dir" && Path::new(profile) == target.profile)
    }
}

fn codex(target: &Target) -> Command {
    let mut command = Command::new("codex");
    command.stdin(Stdio::null());
    let inherited = env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<_> = env::split_paths(&inherited).collect();
    #[cfg(unix)]
    paths.extend([
        target.home.join(".local/bin"),
        target.home.join(".cargo/bin"),
        target.home.join(".npm-global/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    #[cfg(target_os = "macos")]
    paths.push(PathBuf::from("/opt/homebrew/bin"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        if let Some(appdata) = env::var_os("APPDATA") {
            paths.push(PathBuf::from(appdata).join("npm"));
        }
        paths.push(target.home.join(".cargo").join("bin"));
    }
    if let Ok(path) = env::join_paths(paths) {
        command.env("PATH", path);
    }
    command
}

fn configured(target: &Target) -> io::Result<Option<Server>> {
    let result = codex(target).args(["mcp", "list", "--json"]).output()?;
    if !result.status.success() {
        return Err(io::Error::other(
            "could not read Codex CLI MCP configuration",
        ));
    }
    let servers: Vec<Server> = serde_json::from_slice(&result.stdout).map_err(io::Error::other)?;
    Ok(servers.into_iter().find(|server| server.name == SERVER))
}

fn register(executable: &Path, target: &Target) -> io::Result<()> {
    let result = codex(target)
        .args(["mcp", "add", SERVER, "--"])
        .arg(executable)
        .args(["--mcp", "--data-dir"])
        .arg(&target.profile)
        .output()?;
    if result.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            "Codex CLI could not register the MCP server",
        ))
    }
}

fn unregister(target: &Target) -> io::Result<()> {
    let result = codex(target).args(["mcp", "remove", SERVER]).output()?;
    if result.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            "Codex CLI could not remove the MCP server",
        ))
    }
}

fn integration_lock(target: &Target) -> io::Result<fs::File> {
    fs::create_dir_all(&target.profile)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(target.profile.join("codex-integration.lock"))?;
    lock.lock()?;
    Ok(lock)
}

fn managed_server(target: &Target) -> io::Result<Option<Server>> {
    let server = configured(target)?;
    if server
        .as_ref()
        .is_some_and(|server| !server.transport.managed(target))
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a different ndm-next MCP server already exists",
        ));
    }
    Ok(server)
}

impl Harness for CodexCli {
    fn status(&self, target: &Target) -> io::Result<Status> {
        let server = managed_server(target)?;
        Ok(Status {
            registered: server.is_some_and(|server| {
                server.enabled && server.transport.command.as_deref() == Some(&target.executable)
            }),
            skill_installed: skill::is_installed(target)?,
        })
    }

    fn install(&self, target: &Target) -> io::Result<()> {
        if !target.profile.is_absolute()
            || !target.executable.is_absolute()
            || !target.executable.is_file()
        {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "NDM Next host is unavailable",
            ));
        }
        let _lock = integration_lock(target)?;
        let current = managed_server(target)?;
        skill::is_installed(target)?;
        let changed = current.as_ref().is_none_or(|server| {
            !server.enabled || server.transport.command.as_deref() != Some(&target.executable)
        });
        if changed {
            register(&target.executable, target)?;
        }
        if let Err(error) = skill::write(target) {
            if changed {
                if let Some(previous) = current.and_then(|server| server.transport.command) {
                    let _ = register(&previous, target);
                } else {
                    let _ = unregister(target);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn remove(&self, target: &Target) -> io::Result<()> {
        let _lock = integration_lock(target)?;
        let server = managed_server(target)?;
        let owned = skill::is_installed(target)?;
        if server.is_some() {
            unregister(target)?;
        }
        if owned {
            skill::remove(target)?;
        }
        Ok(())
    }
}

impl CodexCli {
    /// Refresh documentation for an existing app-managed installation.
    pub fn sync_skill(&self, target: &Target) -> io::Result<()> {
        if !skill::is_installed(target)? {
            return Ok(());
        }
        let _lock = integration_lock(target)?;
        // Read committed settings after locking so queued refreshes converge.
        if skill::is_installed(target)? {
            skill::write(target)?;
        }
        Ok(())
    }
}
