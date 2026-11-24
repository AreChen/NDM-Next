//! User-scoped MCP and Skill registration for local AI clients.
use std::{env, fs, io, path::PathBuf};

mod config;
mod skill;

const SERVER: &str = "ndm-next";
const MCP_ARGS: [&str; 2] = ["--mcp", "--data-dir"];

#[derive(Default)]
struct Locations {
    codex: Option<PathBuf>,
    claude: Option<PathBuf>,
    pi: Option<PathBuf>,
    opencode: Option<PathBuf>,
    opencode_file: Option<PathBuf>,
}

pub struct Target {
    pub executable: PathBuf,
    pub profile: PathBuf,
    pub home: PathBuf,
    locations: Locations,
}

impl Target {
    pub fn new(executable: PathBuf, profile: PathBuf, home: PathBuf) -> Self {
        Self {
            executable,
            profile,
            home,
            locations: Locations::default(),
        }
    }

    pub fn for_desktop(profile: PathBuf) -> io::Result<Self> {
        let executable = env::current_exe()?.with_file_name(host_name());
        let home = env::home_dir().ok_or_else(|| io::Error::other("user home is unavailable"))?;
        let mut target = Self::new(executable, profile, home);
        target.locations = Locations {
            codex: environment_path("CODEX_HOME"),
            claude: environment_path("CLAUDE_CONFIG_DIR"),
            pi: environment_path("PI_CODING_AGENT_DIR"),
            opencode: environment_path("OPENCODE_CONFIG_DIR")
                .or_else(|| environment_path("XDG_CONFIG_HOME").map(|root| root.join("opencode"))),
            opencode_file: environment_path("OPENCODE_CONFIG"),
        };
        Ok(target)
    }

    fn launch_arguments(&self) -> io::Result<[&str; 3]> {
        let profile = self.profile.to_str().ok_or_else(invalid_config)?;
        Ok([MCP_ARGS[0], MCP_ARGS[1], profile])
    }
}

fn environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn host_name() -> &'static str {
    if cfg!(windows) {
        "ndm-host.exe"
    } else {
        "ndm-host"
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Harness {
    Codex,
    ClaudeCode,
    Pi,
    OpenCode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum McpStatus {
    #[default]
    Missing,
    Configured,
    NeedsRepair,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Status {
    pub mcp: McpStatus,
    pub skill_installed: bool,
}

impl Status {
    pub fn installed(self) -> bool {
        self.mcp == McpStatus::Configured && self.skill_installed
    }

    pub fn has_integration(self) -> bool {
        self.mcp != McpStatus::Missing || self.skill_installed
    }
}

impl Harness {
    pub const ALL: [Self; 4] = [Self::Codex, Self::ClaudeCode, Self::Pi, Self::OpenCode];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Codex => "OpenAI Codex",
            Self::ClaudeCode => "Claude Code",
            Self::Pi => "Pi",
            Self::OpenCode => "OpenCode",
        }
    }

    fn config_directory(self, target: &Target) -> PathBuf {
        let (override_path, default) = match self {
            Self::Codex => (&target.locations.codex, ".codex"),
            Self::ClaudeCode => (&target.locations.claude, ".claude"),
            Self::Pi => (&target.locations.pi, ".pi/agent"),
            Self::OpenCode => (&target.locations.opencode, ".config/opencode"),
        };
        override_path
            .clone()
            .unwrap_or_else(|| target.home.join(default))
    }

    /// The last candidate is the default for new registrations. Existing
    /// OpenCode entries may also live in the older config.json or in JSONC.
    fn config_files(self, target: &Target) -> Vec<PathBuf> {
        let directory = self.config_directory(target);
        match self {
            Self::Codex => vec![directory.join("config.toml")],
            Self::ClaudeCode => vec![
                target
                    .locations
                    .claude
                    .as_ref()
                    .unwrap_or(&target.home)
                    .join(".claude.json"),
            ],
            Self::Pi => vec![directory.join("mcp.json")],
            Self::OpenCode => {
                let mut files = vec![
                    directory.join("config.json"),
                    directory.join("opencode.json"),
                    directory.join("opencode.jsonc"),
                ];
                if let Some(file) = &target.locations.opencode_file {
                    files.retain(|path| path != file);
                    files.push(file.clone());
                }
                files
            }
        }
    }

    fn skill_path(self, target: &Target) -> PathBuf {
        let root = if self == Self::Codex {
            target.home.join(".agents")
        } else {
            self.config_directory(target)
        };
        root.join("skills").join(SERVER).join("SKILL.md")
    }

    pub fn status(self, target: &Target) -> io::Result<Status> {
        let (_, mcp) = self.configuration(target)?;
        Ok(Status {
            mcp,
            skill_installed: skill::is_installed(&self.skill_path(target))?,
        })
    }

    pub fn install(self, target: &Target) -> io::Result<()> {
        if !target.profile.is_absolute()
            || !target.executable.is_absolute()
            || !target.executable.is_file()
        {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "NDM Next host is unavailable",
            ));
        }
        self.change(target, true)
    }

    pub fn remove(self, target: &Target) -> io::Result<()> {
        self.change(target, false)
    }

    fn change(self, target: &Target, install: bool) -> io::Result<()> {
        let _lock = integration_lock(target)?;
        let (mut config, mcp) = self.configuration(target)?;
        let skill_path = self.skill_path(target);
        let skill_installed = skill::is_installed(&skill_path)?;
        let updated = match (install, mcp) {
            (true, McpStatus::Missing | McpStatus::NeedsRepair) => {
                config.register(target, self)?;
                config.save()?
            }
            (false, McpStatus::Configured | McpStatus::NeedsRepair) => {
                config.unregister(self)?;
                config.save()?
            }
            _ => None,
        };
        let result = if install {
            skill::write(&skill_path, &target.profile)
        } else if skill_installed {
            skill::remove(&skill_path)
        } else {
            Ok(())
        };
        if result.is_err()
            && let Some(written) = updated
        {
            config.restore(&written)?;
        }
        result
    }

    fn configuration(self, target: &Target) -> io::Result<(config::Config, McpStatus)> {
        let mut preferred = None;
        let mut existing = None;
        let mut registered = None;
        for path in self.config_files(target) {
            let requested = target.locations.opencode_file.as_ref() == Some(&path);
            let file = config::Config::open(path, self)?;
            let state = file.status(target, self)?;
            if state != McpStatus::Missing {
                if registered.is_some() {
                    return Err(config_conflict());
                }
                registered = Some((file, state));
            } else if file.exists() || requested {
                existing = Some((file, state));
            } else {
                preferred = Some((file, state));
            }
        }
        Ok(registered
            .or(existing)
            .or(preferred)
            .expect("at least one configuration path per AI client"))
    }
}

/// Refresh only the Skill copies that NDM Next already manages.
pub fn sync_skills(target: &Target) -> io::Result<()> {
    let _lock = integration_lock(target)?;
    let mut result = Ok(());
    for client in Harness::ALL {
        let path = client.skill_path(target);
        let update = match skill::is_installed(&path) {
            Ok(true) => skill::write(&path, &target.profile),
            Ok(false) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error),
        };
        if result.is_ok() {
            result = update;
        }
    }
    result
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

fn config_conflict() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "conflicting ndm-next client configuration",
    )
}

fn invalid_config() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid AI client configuration",
    )
}
