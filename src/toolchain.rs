//! The Toolchain Store: pinned toolchains that mise installs on the host, mounted read-only into sandboxes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const RUSTUP: &str = "rustup";
const PIN_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$";

/// Pinned versions, as mise reads them: "24", "24.11" or "24.11.1". Without a pin the sandbox's /usr/bin toolchain is used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(PIN_PATTERN))]
    pub node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(PIN_PATTERN))]
    pub rust: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(PIN_PATTERN))]
    pub python: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(PIN_PATTERN))]
    pub pnpm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(PIN_PATTERN))]
    pub yarn: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Node,
    Rust,
    Python,
    Pnpm,
    Yarn,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Node => "node",
            Tool::Rust => "rust",
            Tool::Python => "python",
            Tool::Pnpm => "pnpm",
            Tool::Yarn => "yarn",
        }
    }
}

impl Toolchain {
    pub fn pins(&self) -> Vec<(Tool, &str)> {
        [
            (Tool::Node, &self.node),
            (Tool::Rust, &self.rust),
            (Tool::Python, &self.python),
            (Tool::Pnpm, &self.pnpm),
            (Tool::Yarn, &self.yarn),
        ]
        .into_iter()
        .filter_map(|(tool, pin)| pin.as_deref().map(|pin| (tool, pin)))
        .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.pins().is_empty()
    }

    /// The first pin that isn't a plain version, as a sentence fragment.
    pub fn problem(&self) -> Option<String> {
        self.pins().into_iter().find_map(|(tool, pin)| {
            let plain = pin.len() <= 64
                && pin.starts_with(|c: char| c.is_ascii_alphanumeric())
                && pin
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
            (!plain).then(|| {
                format!(
                    "toolchain {} is {pin:?}, which is not a version. Use a version such as \"24\" or \"24.11.1\"",
                    tool.name()
                )
            })
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fetch {
    Installed,
    Present,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTool {
    pub tool: Tool,
    pub pin: String,
    pub version: String,
    pub bin_dirs: Vec<PathBuf>,
    pub fetch: Fetch,
}

/// The resolved pins of one Project Profile and the store that holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toolchains {
    pub store: PathBuf,
    pub tools: Vec<ResolvedTool>,
}

impl Toolchains {
    pub fn path_dirs(&self) -> Vec<PathBuf> {
        self.tools
            .iter()
            .flat_map(|tool| tool.bin_dirs.iter().cloned())
            .collect()
    }

    /// PATH for a sandbox: the pinned bin folders ahead of `base`.
    pub fn path(&self, base: &str) -> OsString {
        let mut path = OsString::new();
        for directory in self.path_dirs() {
            path.push(directory);
            path.push(":");
        }
        path.push(base);
        path
    }

    pub fn env(&self) -> Vec<(&'static str, OsString)> {
        let mut env = Vec::new();
        for tool in &self.tools {
            match tool.tool {
                // rustup's proxies find their toolchain through these. CARGO_HOME stays unset, so cargo writes to the sandbox's own HOME.
                Tool::Rust => {
                    env.push(("RUSTUP_HOME", self.store.join(RUSTUP).into()));
                    env.push(("RUSTUP_TOOLCHAIN", tool.version.clone().into()));
                }
                Tool::Node | Tool::Python | Tool::Pnpm | Tool::Yarn => {}
            }
        }
        env
    }
}

pub struct MiseOutput {
    pub success: bool,
    pub stdout: String,
}

pub type MiseFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<MiseOutput>> + Send + 'a>>;

pub trait Mise: Send + Sync {
    /// Runs mise against `store`. With `log`, its output goes there and `stdout` is empty.
    fn run<'a>(
        &'a self,
        store: &'a Path,
        args: &'a [String],
        log: Option<&'a Path>,
    ) -> MiseFuture<'a>;
}

pub struct HostMise;

impl HostMise {
    /// mise with every folder it reads or writes inside `store`, so the developer's own mise, rustup and cargo stay untouched.
    pub(crate) fn command(store: &Path, args: &[String]) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("/usr/bin/mise");
        command
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin")
            .env("HOME", store.join("home"))
            .env("MISE_DATA_DIR", store.join("data"))
            .env("MISE_CONFIG_DIR", store.join("config"))
            .env("MISE_CACHE_DIR", store.join("cache"))
            .env("MISE_STATE_DIR", store.join("state"))
            // An empty folder in place of /etc/mise, whose system config can name tools that then fail to install.
            .env("MISE_SYSTEM_CONFIG_DIR", store.join("system"))
            .env("MISE_GLOBAL_CONFIG_FILE", store.join("config/config.toml"))
            .env("MISE_RUSTUP_HOME", store.join(RUSTUP))
            .env("MISE_CARGO_HOME", store.join("cargo"))
            .env("MISE_CEILING_PATHS", store)
            .env("MISE_YES", "1")
            .current_dir(store.join("work"))
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        command
    }
}

impl Mise for HostMise {
    fn run<'a>(
        &'a self,
        store: &'a Path,
        args: &'a [String],
        log: Option<&'a Path>,
    ) -> MiseFuture<'a> {
        Box::pin(async move {
            let mut command = Self::command(store, args);
            if let Some(log) = log {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log)?;
                command.stdout(file.try_clone()?).stderr(file);
            }
            let output = command.output().await?;
            Ok(MiseOutput {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            })
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolchainError {
    #[error("Cannot use the Toolchain Store at {path}: {source}. Check the folder and try again.", path = .path.display())]
    Store {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "Cannot run mise for {tool} {pin}: {source}. Install mise at /usr/bin/mise, or remove the pin from the Project Profile."
    )]
    Run {
        tool: &'static str,
        pin: String,
        source: std::io::Error,
    },
    #[error("mise could not install {tool} {pin}. Read {log} for the reason, then fix the pin with set_project_profile and try again.", log = .log.display())]
    Install {
        tool: &'static str,
        pin: String,
        log: PathBuf,
    },
    #[error("mise installed {tool} {pin} but did not report {what}. Read {log}, then try again.", log = .log.display())]
    Unreported {
        tool: &'static str,
        pin: String,
        what: &'static str,
        log: PathBuf,
    },
    #[error("mise took over {} minutes to install {tool} {pin}. Check the network and try again.", crate::project::layer::INSTALL_LIMIT.as_secs() / 60)]
    TimedOut { tool: &'static str, pin: String },
}

/// Installs each pin that the store lacks, then finds the version and bin folders of each.
pub async fn resolve(
    mise: &dyn Mise,
    store: &Path,
    toolchain: &Toolchain,
    log: &Path,
) -> Result<Toolchains, ToolchainError> {
    let mut toolchains = Toolchains {
        store: store.to_owned(),
        tools: vec![],
    };
    if toolchain.is_empty() {
        return Ok(toolchains);
    }
    let _lock = lock(store).await?;
    for (tool, pin) in toolchain.pins() {
        toolchains
            .tools
            .push(resolve_one(mise, store, tool, pin, log).await?);
    }
    Ok(toolchains)
}

// One mise at a time per store, so two Projects pinning the same version download it once.
async fn lock(store: &Path) -> Result<std::fs::File, ToolchainError> {
    let path = store.to_owned();
    let store = store.to_owned();
    let failed = |path: &Path| {
        let path = path.to_owned();
        move |source| ToolchainError::Store { path, source }
    };
    tokio::task::spawn_blocking(move || {
        for folder in ["work", "system"] {
            std::fs::create_dir_all(store.join(folder)).map_err(failed(&store))?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(store.join(".lock"))
            .map_err(failed(&store))?;
        lock.lock().map_err(failed(&store))?;
        Ok(lock)
    })
    .await
    .map_err(|error| ToolchainError::Store {
        path,
        source: std::io::Error::other(error),
    })?
}

async fn resolve_one(
    mise: &dyn Mise,
    store: &Path,
    tool: Tool,
    pin: &str,
    log: &Path,
) -> Result<ResolvedTool, ToolchainError> {
    let name = tool.name();
    let ran = |source| ToolchainError::Run {
        tool: name,
        pin: pin.to_owned(),
        source,
    };
    let unreported = |what| ToolchainError::Unreported {
        tool: name,
        pin: pin.to_owned(),
        what,
        log: log.to_owned(),
    };
    let wanted = format!("{name}@{pin}");
    // Before an install, mise resolves a partial pin against the newest remote release, so a miss here is expected.
    let present = mise
        .run(store, &["where".to_owned(), wanted.clone()], None)
        .await
        .map_err(ran)?;
    let (install, fetch) = if present.success {
        (present.stdout, Fetch::Present)
    } else {
        let installed = tokio::time::timeout(
            crate::project::layer::INSTALL_LIMIT,
            mise.run(store, &["install".to_owned(), wanted.clone()], Some(log)),
        )
        .await
        .map_err(|_| ToolchainError::TimedOut {
            tool: name,
            pin: pin.to_owned(),
        })?
        .map_err(ran)?;
        if !installed.success {
            return Err(ToolchainError::Install {
                tool: name,
                pin: pin.to_owned(),
                log: log.to_owned(),
            });
        }
        let found = mise
            .run(store, &["where".to_owned(), wanted], None)
            .await
            .map_err(ran)?;
        if !found.success {
            return Err(unreported("where it is"));
        }
        (found.stdout, Fetch::Installed)
    };
    let version = Path::new(install.trim())
        .file_name()
        .and_then(|version| version.to_str())
        .map(str::to_owned)
        .ok_or_else(|| unreported("its version"))?;
    let paths = mise
        .run(
            store,
            &["bin-paths".to_owned(), format!("{name}@{version}")],
            None,
        )
        .await
        .map_err(ran)?;
    // The sandbox mounts only the store, so a folder outside it would not exist there.
    let bin_dirs: Vec<PathBuf> = paths
        .stdout
        .lines()
        .map(PathBuf::from)
        .filter(|path| path.starts_with(store))
        .collect();
    if !paths.success || bin_dirs.is_empty() {
        return Err(unreported("its bin folders"));
    }
    Ok(ResolvedTool {
        tool,
        pin: pin.to_owned(),
        version,
        bin_dirs,
        fetch,
    })
}

#[cfg(test)]
#[path = "toolchain_tests.rs"]
mod tests;
