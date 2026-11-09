mod common;

use std::fs::File;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, bail};
use common::TestEnv;
use keek::ipc::{Connection, Message, Request, Response};
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

const OLD_VERSION: &str = "0.0.1";
const NEWER_VERSION: &str = "99.0.0";

/// How long a replaced daemon keeps its lock after it answers, so `keek mcp` has to
/// start a daemon again after the first one loses the lock.
const LOCK_HANDOFF: Duration = Duration::from_millis(200);

#[derive(Debug, PartialEq, Eq)]
enum ShutdownAsked {
    Never,
    Accepted,
    Refused,
}

/// Runs a daemon reporting `version`. A fake, because a real daemon reports this
/// build's version, and can't have an open session until Agent Sessions exist.
fn start_fake_daemon(
    env: &TestEnv,
    version: &'static str,
    open_sessions: usize,
) -> Result<JoinHandle<Result<ShutdownAsked>>> {
    let socket = env.socket();
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = File::create(env.lock())?;
    lock.try_lock()?;
    let listener = UnixListener::bind(&socket)?;
    Ok(tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut connection = Connection::new(stream);
        let Some(Message::Hello { .. }) = connection.recv().await? else {
            bail!("the client did not open with hello");
        };
        connection
            .send(&Message::Hello {
                version: version.to_owned(),
                build: None,
            })
            .await?;
        let id = match connection.recv().await? {
            None => return Ok(ShutdownAsked::Never),
            Some(Message::Request {
                id,
                request: Request::Shutdown,
            }) => id,
            Some(other) => bail!("the client sent {other:?}"),
        };
        if open_sessions > 0 {
            let response = Response::ShutdownRefused { open_sessions };
            connection.send(&Message::Response { id, response }).await?;
            return Ok(ShutdownAsked::Refused);
        }
        let response = Response::ShuttingDown;
        connection.send(&Message::Response { id, response }).await?;
        std::fs::remove_file(&socket)?;
        tokio::time::sleep(LOCK_HANDOFF).await;
        drop(lock);
        Ok(ShutdownAsked::Accepted)
    }))
}

async fn run_keek_mcp_to_exit(env: &TestEnv) -> Result<std::process::Output> {
    Ok(tokio::time::timeout(
        Duration::from_secs(10),
        env.keek("mcp").stdin(Stdio::null()).output(),
    )
    .await??)
}

fn assert_names_both_versions(stderr: &str, daemon_version: &str) {
    assert!(stderr.contains(daemon_version), "stderr was: {stderr}");
    assert!(
        stderr.contains(env!("CARGO_PKG_VERSION")),
        "stderr was: {stderr}"
    );
}

#[tokio::test]
async fn with_no_session_open_the_old_daemon_exits_and_a_new_one_starts() -> Result<()> {
    let env = TestEnv::new()?;
    let old_daemon = start_fake_daemon(&env, OLD_VERSION, 0)?;

    let client = env.connect_mcp().await?;

    assert_eq!(old_daemon.await??, ShutdownAsked::Accepted);
    let status = common::keek_status(&client).await?;
    assert_eq!(status.version, env!("CARGO_PKG_VERSION"));
    assert_ne!(status.pid, std::process::id());
    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn with_a_session_open_keek_mcp_refuses_and_names_both_versions() -> Result<()> {
    let env = TestEnv::new()?;
    let old_daemon = start_fake_daemon(&env, OLD_VERSION, 1)?;

    let output = run_keek_mcp_to_exit(&env).await?;

    assert!(!output.status.success(), "keek mcp did not refuse");
    assert_names_both_versions(&String::from_utf8_lossy(&output.stderr), OLD_VERSION);
    assert_eq!(old_daemon.await??, ShutdownAsked::Refused);
    assert!(env.socket().exists());
    Ok(())
}

#[tokio::test]
async fn an_older_keek_mcp_refuses_and_leaves_a_newer_daemon_running() -> Result<()> {
    let env = TestEnv::new()?;
    let newer_daemon = start_fake_daemon(&env, NEWER_VERSION, 0)?;

    let output = run_keek_mcp_to_exit(&env).await?;

    assert!(!output.status.success(), "keek mcp did not refuse");
    assert_names_both_versions(&String::from_utf8_lossy(&output.stderr), NEWER_VERSION);
    assert_eq!(newer_daemon.await??, ShutdownAsked::Never);
    assert!(env.socket().exists());
    Ok(())
}
