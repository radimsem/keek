#![allow(
    dead_code,
    reason = "The shared desktop fixture supports other live tests."
)]
include!("common/live.rs");
include!("common/live_evidence.rs");
mod common;

use std::time::Instant;

const EVIDENCE_WAIT: Duration = Duration::from_secs(60);
const CANCEL_WAIT: Duration = Duration::from_secs(15);

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta, Chromium and pinned Cua Driver"]
async fn live_killing_keek_mcp_files_incomplete_evidence_and_leaves_no_sandbox() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("vanished-agent").await?;
        let session = fixture.open("vanished-agent", &worktree).await?;
        fixture.click(&session, 1).await?;
        kill_keek_mcp().await?;
        let (report, text) = incomplete_report(&fixture, "vanished-agent", EVIDENCE_WAIT).await?;
        println!("{}:\n{text}", report.display());
        assert_incomplete(&text, "keek mcp disconnected")?;
        assert_processes_gone(&session_folder(&fixture, &session)).await?;
        assert_host_unchanged(&before).await
    }
    .await;
    conclude(fixture, result).await
}

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta, Chromium and pinned Cua Driver"]
async fn live_killing_the_daemon_leaves_a_session_the_next_daemon_files() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let first = fixture.daemons[0];
        let worktree = fixture.worktree("fallen-daemon").await?;
        let session = fixture.open("fallen-daemon", &worktree).await?;
        fixture.click(&session, 1).await?;
        let folder = session_folder(&fixture, &session);
        run("kill", &["-KILL", &first.to_string()], None).await?;
        common::wait_until("the killed daemon exits", || common::process_is_gone(first)).await?;
        assert_processes_gone(&folder).await?;
        println!("daemon {first} was killed and took the sandbox with it");

        let second = fixture.reconnect().await?;
        anyhow::ensure!(
            second != first,
            "the new keek mcp reached the killed daemon"
        );
        let (report, text) = incomplete_report(&fixture, "fallen-daemon", EVIDENCE_WAIT).await?;
        println!("{}:\n{text}", report.display());
        assert_incomplete(&text, "daemon stopped")?;
        assert_processes_gone(&folder).await?;
        assert_host_unchanged(&before).await
    }
    .await;
    conclude(fixture, result).await
}

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta, Chromium and pinned Cua Driver"]
async fn live_an_idle_session_ends_as_incomplete() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let mut fixture = Fixture::start_with(&[("KEEK_SESSION_IDLE_MS", "5000")]).await?;
    let result = async {
        let worktree = fixture.worktree("idle-agent").await?;
        let session = fixture.open("idle-agent", &worktree).await?;
        let opened = Instant::now();
        let (report, text) = incomplete_report(&fixture, "idle-agent", EVIDENCE_WAIT).await?;
        println!(
            "{} was filed {:?} after open_session returned:\n{text}",
            report.display(),
            opened.elapsed()
        );
        assert_incomplete(&text, "after 5 seconds without a tool call")?;
        anyhow::ensure!(
            opened.elapsed() >= Duration::from_secs(4),
            "the session ended {:?} after opening, well before its 5 second idle limit",
            opened.elapsed()
        );
        assert_processes_gone(&session_folder(&fixture, &session)).await?;
        assert_host_unchanged(&before).await
    }
    .await;
    conclude(fixture, result).await
}

#[tokio::test]
#[ignore = "requires real bwrap, pasta and pinned Cua Driver"]
async fn live_killing_keek_mcp_during_a_slow_install_leaves_no_sandbox_or_build() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("slow-install").await?;
        write_profile(
            fixture.directory.path(),
            "slow-install",
            "install = \"sleep 60\"\nstart = \"true\"\n",
        )
        .await?;
        let cache = fixture.directory.path().join("cache/keek");
        let sessions = cache.join("sessions");
        let build = cache.join("projects/slow-install/.build");

        let mut opening = Box::pin(call(
            &fixture.client,
            "open_session",
            json!({"project":"slow-install","worktree":worktree,"goal":"verify a slow install","checks":["never reached"]}),
        ));
        let installing = {
            let build = build.clone();
            wait_for("the install to start", EVIDENCE_WAIT, move || {
                let folder = children(&sessions)
                    .into_iter()
                    .find(|folder| folder.join("install.log").exists())?;
                build.is_dir().then_some(folder)
            })
        };
        let folder = tokio::select! {
            outcome = &mut opening => anyhow::bail!("open_session ended while the install still ran: {outcome:?}"),
            folder = installing => folder?,
        };
        // pasta and bwrap start just after install.log is created. Without them
        // running, the checks after the kill would pass for no reason.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let running = assert_processes_gone(&folder).await;
        anyhow::ensure!(
            running.is_err(),
            "no sandbox process was running the install in {}",
            folder.display()
        );
        println!("install sandbox before the kill: {:#}", running.unwrap_err());

        kill_keek_mcp().await?;
        wait_processes_gone(&folder, CANCEL_WAIT).await?;
        wait_for("the build folder to be removed", CANCEL_WAIT, {
            let build = build.clone();
            move || (!build.exists()).then_some(())
        })
        .await
        .with_context(|| format!("{} remains after the cancelled install", build.display()))?;
        let daemon = fixture.daemons[0];
        wait_for("the daemon to exit", CANCEL_WAIT, move || {
            common::process_is_gone(daemon).then_some(())
        })
        .await?;
        let filed = evidence_files(&fixture.directory.path().join("videos"));
        anyhow::ensure!(
            filed.is_empty(),
            "a session that never opened filed Evidence: {filed:?}"
        );
        Ok(())
    }
    .await;
    conclude(fixture, result).await
}

async fn wait_processes_gone(folder: &Path, limit: Duration) -> Result<()> {
    let deadline = Instant::now() + limit;
    loop {
        match assert_processes_gone(folder).await {
            Err(error) if Instant::now() < deadline => println!("still waiting: {error:#}"),
            result => return result,
        }
    }
}

fn assert_incomplete(text: &str, reason: &str) -> Result<()> {
    anyhow::ensure!(
        text.contains(reason),
        "the report does not give the reason {reason:?}:\n{text}"
    );
    anyhow::ensure!(
        !text.contains("## Verdict"),
        "an Incomplete Session report has a Verdict:\n{text}"
    );
    Ok(())
}

fn session_folder(fixture: &Fixture, session: &str) -> PathBuf {
    fixture
        .directory
        .path()
        .join("cache/keek/sessions")
        .join(session)
}

async fn assert_host_unchanged(before: &Value) -> Result<()> {
    let after = host_state().await?;
    anyhow::ensure!(
        *before == after,
        "host focus or workspace changed: before={before}, after={after}"
    );
    Ok(())
}

/// Kills the one `keek mcp` this test process started, without letting it say goodbye.
async fn kill_keek_mcp() -> Result<()> {
    let parent = std::process::id().to_string();
    let output = Command::new("pgrep")
        .args(["-P", &parent, "-f", "keek mcp"])
        .output()
        .await?;
    let pids: Vec<_> = String::from_utf8(output.stdout)?
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    anyhow::ensure!(
        pids.len() == 1,
        "expected one keek mcp child of this test, found {pids:?}"
    );
    run("kill", &["-KILL", &pids[0]], None).await?;
    println!("killed keek mcp pid {}", pids[0]);
    Ok(())
}

async fn diagnostics(root: &Path) -> String {
    let log = root.join("state/keek/daemon.log");
    let journals = root.join("state/keek/sessions");
    let videos = root.join("videos");
    tokio::task::spawn_blocking(move || {
        format!(
            "Evidence files: {:#?}\nSession journals: {:#?}\nDaemon log {}:\n{}",
            evidence_files(&videos),
            children(&journals),
            log.display(),
            std::fs::read_to_string(&log).unwrap_or_else(|error| format!("unreadable: {error}"))
        )
    })
    .await
    .unwrap_or_default()
}

async fn conclude(fixture: Fixture, result: Result<()>) -> Result<()> {
    if result.is_err() {
        println!("{}", diagnostics(fixture.directory.path()).await);
    }
    let stopped = fixture.stop().await;
    if let Err(error) = &stopped {
        println!("daemon cleanup failed: {error:#}");
    }
    result.and(stopped)
}
