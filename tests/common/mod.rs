#![allow(
    dead_code,
    reason = "each test crate uses its own part of these helpers"
)]

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use keek::ipc::DaemonStatus;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use tempfile::TempDir;
use tokio::process::Command;

pub const IDLE_LIMIT_MS: u64 = 300;
pub const FAKE_WAYLAND_DISPLAY: &str = "keek-test-wayland";
pub const FAKE_HYPRLAND_SIGNATURE: &str = "keek-test-hyprland";

pub type McpClient = RunningService<RoleClient, ()>;

/// A throwaway home and runtime folder, so no test touches the developer's daemon.
pub struct TestEnv {
    dir: TempDir,
}

impl TestEnv {
    pub fn new() -> Result<Self> {
        let dir = tempfile::Builder::new().prefix("keek").tempdir()?;
        std::fs::create_dir_all(dir.path().join("run"))?;
        Ok(Self { dir })
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.dir.path().join("run")
    }

    pub fn socket(&self) -> PathBuf {
        self.runtime_dir().join("keek/daemon.sock")
    }

    pub fn lock(&self) -> PathBuf {
        self.runtime_dir().join("keek/daemon.lock")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    pub fn keek(&self, subcommand: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_keek"));
        command
            .arg(subcommand)
            .env_clear()
            .env("HOME", self.dir.path())
            .env("XDG_RUNTIME_DIR", self.runtime_dir())
            .env("XDG_STATE_HOME", self.state_dir())
            .env("KEEK_DAEMON_IDLE_MS", IDLE_LIMIT_MS.to_string())
            .env("WAYLAND_DISPLAY", FAKE_WAYLAND_DISPLAY)
            .env("HYPRLAND_INSTANCE_SIGNATURE", FAKE_HYPRLAND_SIGNATURE)
            .kill_on_drop(true);
        command
    }

    pub async fn connect_mcp(&self) -> Result<McpClient> {
        let mut command = self.keek("mcp");
        command.stderr(Stdio::inherit());
        let transport = TokioChildProcess::new(command)?;
        Ok(().serve(transport).await?)
    }
}

pub async fn keek_status(client: &McpClient) -> Result<DaemonStatus> {
    let result = client
        .call_tool(CallToolRequestParams::new("keek_status"))
        .await?;
    let status = result
        .structured_content
        .context("keek_status returned no structured content")?;
    Ok(serde_json::from_value(status)?)
}

/// A process that has exited but not been reaped still has a `/proc` entry, which
/// counts as gone.
pub fn process_is_gone(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return true;
    };
    stat.rsplit(')')
        .next()
        .is_some_and(|rest| rest.trim_start().starts_with('Z'))
}

pub async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) -> Result<()> {
    for _ in 0..200 {
        if condition() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!("timed out waiting until {what}");
}
