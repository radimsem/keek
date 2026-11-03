#![allow(
    dead_code,
    reason = "The shared desktop fixture supports other live tests."
)]
include!("common/live.rs");
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
    let mut daemons = Vec::new();
    let result = async {
        daemons.push(common::keek_status(&fixture.client).await?.pid);
        let worktree = fixture.worktree("vanished-agent").await?;
        let session = fixture.open("vanished-agent", &worktree).await?;
        fixture.click(&session, 1).await?;
        kill_keek_mcp().await?;
        let (report, text) = incomplete_report(&fixture, "vanished-agent").await?;
        println!("{}:\n{text}", report.display());
        assert_incomplete(&text, "keek mcp disconnected")?;
        assert_processes_gone(&session_folder(&fixture, &session)).await?;
        assert_host_unchanged(&before).await
    }
    .await;
    conclude(fixture, Vec::new(), &daemons, result).await
}

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta, Chromium and pinned Cua Driver"]
async fn live_killing_the_daemon_leaves_a_session_the_next_daemon_files() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let mut fixture = Fixture::start().await?;
    let mut daemons = Vec::new();
    let mut later_clients = Vec::new();
    let result = async {
        let first = common::keek_status(&fixture.client).await?.pid;
        daemons.push(first);
        let worktree = fixture.worktree("fallen-daemon").await?;
        let session = fixture.open("fallen-daemon", &worktree).await?;
        fixture.click(&session, 1).await?;
        let folder = session_folder(&fixture, &session);
        run("kill", &["-KILL", &first.to_string()], None).await?;
        common::wait_until("the killed daemon exits", || common::process_is_gone(first)).await?;
        assert_processes_gone(&folder).await?;
        println!("daemon {first} was killed and took the sandbox with it");

        let transport = TokioChildProcess::new(mcp_command(fixture.directory.path(), &[])?)?;
        later_clients.push(().serve(transport).await?);
        let second = common::keek_status(&later_clients[0]).await?.pid;
        daemons.push(second);
        anyhow::ensure!(
            second != first,
            "the new keek mcp reached the killed daemon"
        );
        let (report, text) = incomplete_report(&fixture, "fallen-daemon").await?;
        println!("{}:\n{text}", report.display());
        assert_incomplete(&text, "daemon stopped")?;
        assert_processes_gone(&folder).await?;
        assert_host_unchanged(&before).await
    }
    .await;
    conclude(fixture, later_clients, &daemons, result).await
}

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta, Chromium and pinned Cua Driver"]
async fn live_an_idle_session_ends_as_incomplete() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let mut fixture = Fixture::start_with(&[("KEEK_SESSION_IDLE_MS", "5000")]).await?;
    let mut daemons = Vec::new();
    let result = async {
        daemons.push(common::keek_status(&fixture.client).await?.pid);
        let worktree = fixture.worktree("idle-agent").await?;
        let session = fixture.open("idle-agent", &worktree).await?;
        let opened = Instant::now();
        let (report, text) = incomplete_report(&fixture, "idle-agent").await?;
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
    conclude(fixture, Vec::new(), &daemons, result).await
}

#[tokio::test]
#[ignore = "requires real bwrap, pasta and pinned Cua Driver"]
async fn live_killing_keek_mcp_during_a_slow_install_leaves_no_sandbox_or_build() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let fixture = Fixture::start().await?;
    let mut daemons = Vec::new();
    let result = async {
        daemons.push(common::keek_status(&fixture.client).await?.pid);
        let worktree = fixture.worktree("slow-install").await?;
        let script = worktree.join(".keek/setup.sh");
        tokio::task::spawn_blocking(move || {
            std::fs::write(script, "#!/bin/bash\ncase \"$1\" in\ninstall) sleep 60 ;;\nesac\n")
        })
        .await??;
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
        let daemon = daemons[0];
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
    conclude(fixture, Vec::new(), &daemons, result).await
}

async fn wait_for<T: Send + 'static>(
    what: &str,
    limit: Duration,
    probe: impl Fn() -> Option<T> + Clone + Send + 'static,
) -> Result<T> {
    let deadline = Instant::now() + limit;
    loop {
        let probe = probe.clone();
        if let Some(found) = tokio::task::spawn_blocking(probe).await? {
            return Ok(found);
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out after {limit:?} waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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

fn children(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
}

/// Every filed file under `<videos>/keek/<project>/`, including the date folders' contents.
fn evidence_files(videos: &Path) -> Vec<PathBuf> {
    children(&videos.join("keek"))
        .iter()
        .flat_map(|project| children(project))
        .flat_map(|entry| {
            if entry.is_dir() {
                children(&entry)
            } else {
                vec![entry]
            }
        })
        .collect()
}

async fn incomplete_report(fixture: &Fixture, project: &str) -> Result<(PathBuf, String)> {
    let videos = fixture.directory.path().join("videos");
    let prefix = format!("{project}_");
    wait_for(
        "the Incomplete Session's report",
        EVIDENCE_WAIT,
        move || {
            let report = evidence_files(&videos).into_iter().find(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                name.starts_with(&prefix) && name.ends_with("_incomplete_report.md")
            })?;
            // The report is created before it is written.
            let text = std::fs::read_to_string(&report).ok()?;
            text.contains("## Incomplete Session")
                .then_some((report, text))
        },
    )
    .await
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

/// Prints the daemon log after a failure, then stops the daemons so none outlives the temp root.
async fn conclude(
    fixture: Fixture,
    later_clients: Vec<Client>,
    daemons: &[u32],
    result: Result<()>,
) -> Result<()> {
    if result.is_err() {
        println!("{}", diagnostics(fixture.directory.path()).await);
    }
    let Fixture {
        directory, client, ..
    } = fixture;
    for client in std::iter::once(client).chain(later_clients) {
        // A killed keek mcp has nothing left to cancel.
        let _ = client.cancel().await;
    }
    for &daemon in daemons {
        // A daemon that ended a session keeps its End Card until the developer
        // dismisses it, so it does not exit on its own.
        let exit = wait_for("the daemon to exit", Duration::from_secs(5), move || {
            common::process_is_gone(daemon).then_some(())
        });
        if exit.await.is_err() {
            run("kill", &["-KILL", &daemon.to_string()], None).await?;
        }
    }
    drop(directory);
    result
}
