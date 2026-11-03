#![allow(
    dead_code,
    reason = "each test crate uses its own part of these helpers"
)]

use std::path::{Path, PathBuf};
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

pub async fn stop_daemon(runtime: &Path, pid: u32, grace: Duration) -> Result<()> {
    if !identified_daemon(runtime, pid).await? {
        return Ok(());
    }
    // The daemon may already have exited; ignore the kill status.
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .await?;
    let deadline = tokio::time::Instant::now() + grace;
    loop {
        // An environ read fails while the process execs, so only stat says it exited.
        if tokio::task::spawn_blocking(move || process_is_gone(pid)).await? {
            println!("daemon {pid}: stopped by SIGTERM");
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + Duration::from_millis(25)),
        )
        .await;
    }
    if !identified_daemon(runtime, pid).await? {
        return Ok(());
    }
    // The daemon may already have exited; ignore the kill status.
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .await?;
    for _ in 0..200 {
        if tokio::task::spawn_blocking(move || process_is_gone(pid)).await? {
            println!("daemon {pid}: stopped by SIGKILL");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!("timed out waiting until the daemon exits after SIGKILL");
}

async fn identified_daemon(runtime: &Path, pid: u32) -> Result<bool> {
    let runtime = runtime.to_owned();
    Ok(tokio::task::spawn_blocking(move || {
        if is_daemon_of(&runtime, pid) {
            true
        } else {
            if !process_is_gone(pid) {
                println!("daemon {pid}: not this test's daemon, left running");
            }
            false
        }
    })
    .await?)
}

fn is_daemon_of(runtime: &Path, pid: u32) -> bool {
    let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    let mut expected = std::ffi::OsString::from("XDG_RUNTIME_DIR=");
    expected.push(runtime);
    environ
        .split(|&byte| byte == 0)
        .any(|entry| entry == expected.as_encoded_bytes())
}
