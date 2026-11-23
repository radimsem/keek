#![allow(
    dead_code,
    reason = "The shared desktop fixture also supports recording tests."
)]
mod common;
include!("common/live.rs");
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::AsyncReadExt;

struct HostListener {
    port: u16,
    installs: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl HostListener {
    async fn start() -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let installs = Arc::new(AtomicUsize::new(0));
        let count = installs.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                if let Ok(Ok(length)) =
                    tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut bytes))
                        .await
                    && length == 8
                    && bytes == b"install\n"
                {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        Ok(Self {
            port,
            installs,
            task,
        })
    }
}
impl Drop for HostListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn project_open(
    fixture: &mut Fixture,
    project: &str,
    worktree: &Path,
) -> Result<(String, Value)> {
    let response=call(&fixture.client,"open_session",json!({"project":project,"worktree":worktree,"goal":"verify Project isolation","checks":["Project probes match"]})).await?;
    let name = response["session"]
        .as_str()
        .context("session missing")?
        .to_owned();
    fixture.sessions.push(name.clone());
    Ok((name, response))
}
async fn probes(fixture: &Fixture, session: &str) -> Result<Value> {
    let path = fixture
        .directory
        .path()
        .join("cache/keek/sessions")
        .join(session)
        .join("probes.txt");
    for _ in 0..150 {
        let read_path = path.clone();
        if let Ok(Ok(bytes)) = tokio::task::spawn_blocking(move || std::fs::read(read_path)).await {
            return Ok(serde_json::from_slice(&bytes)?);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    bail!("Project probes did not appear at {}", path.display())
}
async fn status(worktree: &Path) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "-z"])
        .current_dir(worktree)
        .output()
        .await?;
    anyhow::ensure!(output.status.success(), "git status failed");
    Ok(output.stdout)
}
async fn clients() -> Result<Value> {
    Ok(serde_json::from_str(
        &run("hyprctl", &["clients", "-j"], None).await?,
    )?)
}
fn addresses(clients: &Value) -> Vec<&str> {
    let mut addresses: Vec<_> = clients
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|client| client["address"].as_str())
        .collect();
    addresses.sort_unstable();
    addresses
}

include!("common/project.rs");

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta, Python, Chromium and pinned Cua Driver"]
async fn live_project_reuses_layers_and_applies_isolation_settings_and_warnings() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let config = host_config().await?;
    let windows = clients().await?;
    let mut fixture = Fixture::start().await?;
    let forwarded = HostListener::start().await?;
    let blocked = HostListener::start().await?;
    let result = async {
        let worktree = fixture.worktree("project").await?;
        let root = worktree.clone();
        let script = fixture_script(forwarded.port, blocked.port);
        tokio::task::spawn_blocking(move || -> Result<()> {
            std::fs::write(root.join("fixture.sh"), script)?;
            std::fs::write(root.join("uv.lock"), "first")?;
            std::fs::write(root.join("source"), "old source")?;
            std::fs::write(root.join(".gitignore"), ".env\n")?;
            std::fs::write(root.join(".env"), "FAKE_ACCOUNT=throwaway\n")?;
            std::fs::write(root.join("untracked"), "untracked content")?;
            std::fs::write(
                root.join("index.html"),
                "<title>keek fixture</title><body>Project fixture</body>",
            )?;
            Ok(())
        })
        .await??;
        run(
            "git",
            &["add", "fixture.sh", "uv.lock", "source", ".gitignore", "index.html"],
            Some(&worktree),
        )
        .await?;
        let settings = fixture.directory.path().join("config/keek/projects");
        let settings_directory = settings.clone();
        tokio::task::spawn_blocking(move || std::fs::create_dir_all(settings_directory)).await??;
        write(
            settings.join("project.toml"),
            format!(
                "install = \"bash fixture.sh install\"\nstart = \"bash fixture.sh start\"\nports = [{}]\n[window]\nwidth = 390\nheight = 844\n",
                forwarded.port
            ),
        )
        .await?;
        write(settings.join("project.env"), b"FAKE_USER=throwaway\n").await?;
        std::fs::set_permissions(
            settings.join("project.env"),
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )?;
        let original_status = status(&worktree).await?;
        let mut keys: Vec<std::ffi::OsString> = Vec::new();
        for (index, expected_count, source) in [
            (0, 1, "old source"),
            (1, 1, "new source"),
            (2, 2, "new source"),
        ] {
            if index == 1 {
                write(worktree.join("source"), "new source").await?;
            }
            if index == 2 {
                write(worktree.join("uv.lock"), "second").await?;
            }
            let expected_status = status(&worktree).await?;
            let (session, response) = project_open(&mut fixture, "project", &worktree).await?;
            anyhow::ensure!(
                response["warnings"] == json!([]),
                "registered Project has warnings"
            );
            if index == 0 {
                fixture.click(&session, 1).await?;
            }
            let probe = probes(&fixture, &session).await?;
            anyhow::ensure!(
                probe["install_cwd"] == probe["cwd"] && probe["cwd"] == "/keek/worktree",
                "install and start paths differ: {probe}"
            );
            anyhow::ensure!(
                probe["project"] == "project" && probe["counter"] == 1,
                "Project or counter probe differs: {probe}"
            );
            anyhow::ensure!(
                probe["ignored"] == false && probe["untracked"] == "untracked content",
                "file visibility differs: {probe}"
            );
            anyhow::ensure!(
                probe["marker"] == false && probe["source"] == source,
                "session writes leaked or source is stale: {probe}"
            );
            anyhow::ensure!(
                probe["ports"] == json!([true, false]),
                "port forwarding differs: {probe}"
            );
            let lan = &probe["lan"];
            anyhow::ensure!(
                lan["blocked_routes"].as_array().is_some_and(
                    |v| v.len() == 7 && v.iter().all(|s| s.as_i64().is_some_and(|n| n != 0))
                ) && lan["routed"] == json!([0, 0, 0])
                    && lan["connects"] == json!([true, true, true, true, true, true, true])
                    && lan["dns"] == true
                    && lan["delete"].as_i64().is_some_and(|n| n != 0),
                "LAN isolation differs: {lan}"
            );
            let current = clients().await?;
            let initial = addresses(&windows);
            let window = current
                .as_array()
                .context("clients missing")?
                .iter()
                .find(|client| {
                    client["address"]
                        .as_str()
                        .is_some_and(|a| !initial.contains(&a))
                        && client["class"] == "aquamarine"
                })
                .context("Session Window not found")?;
            anyhow::ensure!(
                window["size"] == json!([390, 844]),
                "Session Window has wrong size: {window}"
            );
            println!(
                "open {}: host installs={}, in-session counter={}",
                index + 1,
                forwarded.installs.load(Ordering::SeqCst),
                probe["counter"]
            );
            anyhow::ensure!(
                forwarded.installs.load(Ordering::SeqCst) == expected_count,
                "Project Layer rebuild count differs"
            );
            let records = fixture.close(&session).await?;
            let dimensions = run(
                "ffprobe",
                &[
                    "-v",
                    "error",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=width,height",
                    "-of",
                    "csv=s=x:p=0",
                    records
                        .raw
                        .to_str()
                        .context("recording path is not UTF-8")?,
                ],
                None,
            )
            .await?;
            anyhow::ensure!(
                dimensions.trim() == "390x844",
                "Raw Recording dimensions differ: {dimensions}"
            );
            let cache = fixture.directory.path().join("cache/keek/projects/project");
            let mut entries: Vec<_> = std::fs::read_dir(&cache)?
                .map(|entry| entry.map(|e| e.file_name()))
                .collect::<std::io::Result<_>>()?;
            entries.sort();
            if index == 0 || index == 2 {
                let new: Vec<_> = entries
                    .iter()
                    .filter(|name| {
                        *name != ".lock" && *name != ".used" && !keys.contains(*name)
                    })
                    .cloned()
                    .collect();
                anyhow::ensure!(
                    new.len() == 1,
                    "expected one new Project Layer: {entries:?}"
                );
                keys.extend(new);
            }
            let mut expected = keys.clone();
            expected.extend([".lock".into(), ".used".into()]);
            expected.sort();
            anyhow::ensure!(
                entries == expected,
                "unexpected Project cache entries: {entries:?}"
            );
            let trees = fixture.directory.path().join("cache/keek/trees");
            anyhow::ensure!(
                std::fs::read_dir(trees)?.next().is_none(),
                "worktree copies remain after close"
            );
            if index == 0 {
                let second = fixture.directory.path().join("second-worktree");
                run(
                    "git",
                    &[
                        "worktree",
                        "add",
                        "--detach",
                        second.to_str().context("worktree path")?,
                    ],
                    Some(&worktree),
                )
                .await?;
                for name in [
                    "fixture.sh",
                    "uv.lock",
                    "source",
                    ".gitignore",
                    "index.html",
                    "untracked",
                ] {
                    std::fs::copy(worktree.join(name), second.join(name))?;
                }
                run(
                    "git",
                    &["add", "fixture.sh", "uv.lock", "source", ".gitignore", "index.html"],
                    Some(&second),
                )
                .await?;
                let (other, _) = project_open(&mut fixture, "project", &second).await?;
                let other_probe = probes(&fixture, &other).await?;
                anyhow::ensure!(
                    other_probe["install_cwd"] == other_probe["cwd"]
                        && other_probe["cwd"] == "/keek/worktree",
                    "second worktree path differs: {other_probe}"
                );
                anyhow::ensure!(
                    forwarded.installs.load(Ordering::SeqCst) == 1,
                    "second worktree rebuilt Project Layer"
                );
                fixture.close(&other).await?;
            }
            anyhow::ensure!(
                status(&worktree).await? == expected_status,
                "Agent Session changed git status bytes"
            );
            if index == 0 {
                anyhow::ensure!(expected_status == original_status, "initial status differs");
            }
        }
        let refusal = project_open(&mut fixture, "proejct", &worktree)
            .await
            .unwrap_err()
            .to_string();
        anyhow::ensure!(
            refusal.contains("proejct") && refusal.contains("set_project_profile"),
            "a misspelled Project did not refuse with set_project_profile: {refusal}"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    let after = host_state().await?;
    let after_config = host_config().await?;
    let after_windows = clients().await?;
    anyhow::ensure!(
        before == after,
        "host focus or workspace changed: before={before:?}, after={after:?}"
    );
    anyhow::ensure!(config == after_config, "host config changed");
    anyhow::ensure!(
        addresses(&windows) == addresses(&after_windows),
        "host open windows changed"
    );
    result.and(cleanup)
}

#[tokio::test]
#[ignore = "requires real bwrap, pasta and pinned Cua Driver"]
async fn live_project_failed_install_refuses_with_the_end_of_its_log() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let mut fixture = Fixture::start().await?;
    let result=async {
        let worktree=fixture.worktree("failed-project").await?;
        write(worktree.join("fixture.sh"),"#!/bin/bash\nprintf 'head-token\\n'\npython3 -c 'print(\"x\" * 5000)'\nprintf 'tail-token\\n'\nexit 7\n").await?;
        run("git", &["add", "fixture.sh"], Some(&worktree)).await?;
        write_profile(fixture.directory.path(), "failed-project", "install = \"bash fixture.sh\"\nstart = \"true\"\n").await?;
        let error=project_open(&mut fixture,"failed-project",&worktree).await.unwrap_err().to_string();
        anyhow::ensure!(error.contains("status 7")&&error.contains("tail-token")&&!error.contains("head-token")&&error.contains("install.log"),"install refusal did not quote only the log tail");
        Ok::<(),anyhow::Error>(())
    }.await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta and pinned Cua Driver"]
async fn live_project_refuses_a_repository_that_moves_its_worktree() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let other = fixture.worktree("other-project").await?;
        write(other.join("only-in-other-worktree"), "made up\n").await?;
        let hostile = fixture.worktree("hostile").await?;
        let target = other.to_str().context("worktree path")?;
        run("git", &["config", "core.worktree", target], Some(&hostile)).await?;
        match project_open(&mut fixture, "hostile", &hostile).await {
            Ok((session, _)) => {
                let copied = fixture
                    .directory
                    .path()
                    .join("cache/keek/trees")
                    .join(&session)
                    .join("only-in-other-worktree")
                    .exists();
                fixture.close(&session).await?;
                anyhow::bail!(
                    "open_session accepted core.worktree, the other worktree's file was copied: {copied}"
                )
            }
            Err(error) => anyhow::ensure!(
                error.to_string().contains("core.worktree"),
                "refusal does not name core.worktree: {error}"
            ),
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}

async fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref().to_owned();
    let contents = contents.as_ref().to_vec();
    tokio::task::spawn_blocking(move || std::fs::write(path, contents)).await??;
    Ok(())
}
