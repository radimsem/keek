mod common;

use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use common::TestEnv;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[tokio::test]
async fn stops_a_connected_daemon_and_removes_its_socket() -> Result<()> {
    let env = TestEnv::new()?;
    let client = env.connect_mcp().await?;
    let pid = common::keek_status(&client).await?.pid;
    common::stop_daemon(&env.runtime_dir(), pid, Duration::from_secs(5)).await?;
    assert!(
        common::process_is_gone(pid),
        "the connected daemon survived"
    );
    assert!(!env.socket().exists(), "the daemon socket remains");
    let _ = client.cancel().await;
    Ok(())
}

#[tokio::test]
async fn leaves_a_process_with_a_different_runtime_running() -> Result<()> {
    let env = TestEnv::new()?;
    let other = TestEnv::new()?;
    let mut child = Command::new("sleep")
        .arg("60")
        .env("XDG_RUNTIME_DIR", other.runtime_dir())
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id().context("sleep has no pid")?;
    common::stop_daemon(&env.runtime_dir(), pid, Duration::from_millis(300)).await?;
    assert!(
        child.try_wait()?.is_none(),
        "the unrelated process was stopped"
    );
    child.kill().await?;
    Ok(())
}

#[tokio::test]
async fn kills_a_matching_process_that_ignores_sigterm() -> Result<()> {
    let env = TestEnv::new()?;
    let mut child = Command::new("sh")
        .args([
            "-c",
            "trap '' TERM; printf 'ready\\n'; while :; do sleep 1; done",
        ])
        .env("XDG_RUNTIME_DIR", env.runtime_dir())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id().context("shell has no pid")?;
    let mut ready = String::new();
    BufReader::new(child.stdout.take().context("shell has no stdout")?)
        .read_line(&mut ready)
        .await?;
    assert_eq!(ready, "ready\n");
    let started = Instant::now();
    common::stop_daemon(&env.runtime_dir(), pid, Duration::from_millis(300)).await?;
    assert!(
        common::process_is_gone(pid),
        "the TERM-ignoring process survived"
    );
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(child.wait().await?.signal(), Some(9));
    Ok(())
}
