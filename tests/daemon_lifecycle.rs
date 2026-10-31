mod common;

use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use common::{FAKE_HYPRLAND_SIGNATURE, FAKE_WAYLAND_DISPLAY, IDLE_LIMIT_MS, TestEnv};
use keek::ipc::{Connection, Message, Request, RequestId, Response};
use tokio::net::UnixStream;
use tokio::process::Command;

#[tokio::test]
async fn two_mcp_processes_share_one_daemon_that_exits_after_both_leave() -> Result<()> {
    let env = TestEnv::new()?;
    let (first, second) = tokio::try_join!(env.connect_mcp(), env.connect_mcp())?;

    let pid = common::keek_status(&first).await?.pid;
    assert_eq!(common::keek_status(&second).await?.pid, pid);

    first.cancel().await?;
    tokio::time::sleep(Duration::from_millis(IDLE_LIMIT_MS * 2)).await;
    assert_eq!(
        common::keek_status(&second).await?.pid,
        pid,
        "the daemon must stay while a client remains"
    );

    // The daemon can't see the last client leave before this reading, so it can't
    // honestly exit less than the idle limit after it.
    let before_last_client_left = Instant::now();
    second.cancel().await?;
    common::wait_until("the daemon exits", || common::process_is_gone(pid)).await?;

    assert!(
        before_last_client_left.elapsed() >= Duration::from_millis(IDLE_LIMIT_MS),
        "the daemon exited before the idle limit passed"
    );
    assert!(!env.socket().exists(), "the daemon left its socket behind");
    Ok(())
}

#[tokio::test]
async fn a_killed_daemon_is_replaced_despite_its_stale_socket() -> Result<()> {
    let env = TestEnv::new()?;
    let survivor = env.connect_mcp().await?;
    let killed = common::keek_status(&survivor).await?.pid;

    let kill = Command::new("sh")
        .args(["-c", &format!("kill -KILL {killed}")])
        .status()
        .await?;
    assert!(kill.success());
    common::wait_until("the daemon dies", || common::process_is_gone(killed)).await?;
    assert!(env.socket().exists(), "SIGKILL should leave the socket");

    let replacement = env.connect_mcp().await?;

    assert_ne!(common::keek_status(&replacement).await?.pid, killed);
    replacement.cancel().await?;
    drop(survivor);
    Ok(())
}

#[tokio::test]
async fn a_daemon_that_fails_to_start_is_reported_with_its_cause_in_the_log() -> Result<()> {
    let env = TestEnv::new()?;

    let output = env
        .keek("mcp")
        .env("KEEK_DAEMON_IDLE_MS", "soon")
        .stdin(Stdio::null())
        .output()
        .await?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "keek mcp did not fail");
    assert!(stderr.contains("Could not start"), "stderr was: {stderr}");
    assert!(stderr.contains("daemon.log"), "stderr was: {stderr}");
    let log = std::fs::read_to_string(env.state_dir().join("keek/daemon.log"))?;
    assert!(log.contains("soon"), "log was: {log}");
    Ok(())
}

#[tokio::test]
async fn a_second_daemon_exits_and_leaves_the_first_running() -> Result<()> {
    let env = TestEnv::new()?;
    let client = env.connect_mcp().await?;
    let pid = common::keek_status(&client).await?.pid;

    let second = env.keek("daemon").status().await?;

    assert!(second.success(), "second daemon exited with {second}");
    assert_eq!(common::keek_status(&client).await?.pid, pid);
    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn the_daemon_inherits_the_host_environment_from_keek_mcp() -> Result<()> {
    let env = TestEnv::new()?;
    let client = env.connect_mcp().await?;
    let pid = common::keek_status(&client).await?.pid;

    let environ = std::fs::read_to_string(format!("/proc/{pid}/environ"))?;
    let vars: Vec<&str> = environ.split('\0').collect();
    let runtime = format!("XDG_RUNTIME_DIR={}", env.runtime_dir().display());
    let wayland = format!("WAYLAND_DISPLAY={FAKE_WAYLAND_DISPLAY}");
    let hyprland = format!("HYPRLAND_INSTANCE_SIGNATURE={FAKE_HYPRLAND_SIGNATURE}");

    for expected in [&runtime, &wayland, &hyprland] {
        assert!(vars.contains(&expected.as_str()), "missing {expected}");
    }
    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn the_daemon_writes_its_log_under_the_state_folder() -> Result<()> {
    let env = TestEnv::new()?;
    let client = env.connect_mcp().await?;
    common::keek_status(&client).await?;

    let log = std::fs::read_to_string(env.state_dir().join("keek/daemon.log"))?;

    assert!(log.contains(env!("CARGO_PKG_VERSION")), "log was: {log}");
    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn a_daemon_with_no_sessions_accepts_a_shutdown_request() -> Result<()> {
    let env = TestEnv::new()?;
    let client = env.connect_mcp().await?;
    let pid = common::keek_status(&client).await?.pid;

    let stream = UnixStream::connect(env.socket()).await?;
    let mut old_client = Connection::new(stream);
    old_client
        .send(&Message::Hello {
            version: "0.0.1".to_owned(),
        })
        .await?;
    let hello = old_client.recv().await?.context("no hello")?;
    assert_eq!(
        hello,
        Message::Hello {
            version: env!("CARGO_PKG_VERSION").to_owned()
        }
    );
    let id = RequestId(1);
    old_client
        .send(&Message::Request {
            id,
            request: Request::Shutdown,
        })
        .await?;

    assert_eq!(
        old_client.recv().await?,
        Some(Message::Response {
            id,
            response: Response::ShuttingDown,
        })
    );
    common::wait_until("the daemon exits", || common::process_is_gone(pid)).await?;
    assert!(!env.socket().exists());
    drop(client);
    Ok(())
}
