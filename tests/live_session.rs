mod common;
include!("common/live.rs");

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver, Chromium and ffprobe"]
async fn live_session_records_three_clicks() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let config = host_config().await?;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("three-clicks").await?;
        let session = fixture.open("three-clicks", &worktree).await?;
        let pid = fixture.click(&session, 3).await?;
        let folder = fixture.close(&session).await?;
        assert_records(&folder, 3, pid).await?;
        assert_verification_video(&folder.verification).await?;
        assert_processes_gone(&folder.cache).await?;
        let after = host_state().await?;
        println!("host state before={before}, after={after}");
        anyhow::ensure!(
            before == after,
            "host focus or workspace changed: before={before}, after={after}"
        );
        let after_config = host_config().await?;
        println!(
            "host config checked: {} files before, {} after",
            config.len(),
            after_config.len()
        );
        anyhow::ensure!(config == after_config, "host Hyprland config changed");
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result?;
    cleanup
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver, Chromium and ffprobe"]
async fn live_session_two_worktrees_stay_separate() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let first_worktree = fixture.worktree("first").await?;
        let second_worktree = fixture.worktree("second").await?;
        let first = fixture.open("first", &first_worktree).await?;
        let second = fixture.open("second", &second_worktree).await?;
        let first_pid = fixture.click(&first, 1).await?;
        let second_pid = fixture.click(&second, 2).await?;
        let first_folder = fixture.close(&first).await?;
        let second_folder = fixture.close(&second).await?;
        anyhow::ensure!(
            first_folder.report != second_folder.report,
            "sessions share a folder"
        );
        println!(
            "separate sessions: {} and {}",
            first_folder.report.display(),
            second_folder.report.display()
        );
        assert_records(&first_folder, 1, first_pid).await?;
        assert_records(&second_folder, 2, second_pid).await?;
        assert_processes_gone(&first_folder.cache).await?;
        assert_processes_gone(&second_folder.cache).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result?;
    cleanup
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver and Chromium"]
async fn live_evidence() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("evidence").await?;
        let session = fixture.open("myapp", &worktree).await?;
        fixture.click(&session, 1).await?;
        call(&fixture.client,"mark_check",json!({"session":session,"check":1,"outcome":"passed"})).await?;
        let failed=call(&fixture.client,"add_check",json!({"session":session,"text":"fixture reports errors"})).await?;
        call(&fixture.client,"mark_check",json!({"session":session,"check":failed["check"],"outcome":"failed","comment":"Fixture has no error display"})).await?;
        let skipped=call(&fixture.client,"add_check",json!({"session":session,"text":"fixture keyboard input"})).await?;
        call(&fixture.client,"mark_check",json!({"session":session,"check":skipped["check"],"outcome":"skipped","comment":"Fixture has no input field"})).await?;
        let records=fixture.close_marked(&session).await?;
        anyhow::ensure!(records.response["verdict"]=="Needs fixes","unexpected Verdict");
        let videos=fixture.directory.path().join("videos");
        for key in ["report","raw","verification","timeline","actions","index"] {
            let path=Path::new(records.response["evidence"][key].as_str().context("missing Evidence path")?);
            anyhow::ensure!(path.starts_with(&videos),"Evidence escaped temp videos folder");
        }
        let cache=records.cache;
        let report=records.report;
        tokio::task::spawn_blocking(move || -> Result<()> {
            fn print_tree(path:&Path) -> Result<()> {
                println!("{}",path.display());
                if path.is_dir() { for entry in std::fs::read_dir(path)? { print_tree(&entry?.path())?; } }
                Ok(())
            }
            print_tree(&videos)?;
            println!("{}",std::fs::read_to_string(report)?);
            anyhow::ensure!(!cache.exists(),"session cache folder remains");
            Ok(())
        }).await??;
        Ok(())
    }.await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}

async fn assert_verification_video(path: &Path) -> Result<()> {
    let duration = run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path.to_str()
                .context("Verification Video path is not UTF-8")?,
        ],
        None,
    )
    .await?
    .parse::<f64>()?;
    println!(
        "{}: ffprobe Verification Video duration={duration}s",
        path.display()
    );
    anyhow::ensure!(duration > 0.0, "Verification Video duration is zero");
    Ok(())
}

#[tokio::test]
#[ignore = "requires Hyprland, pinned Cua Driver and Chromium; start on another workspace"]
async fn live_viewer_open_session_preserves_host_focus_and_workspace() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    anyhow::ensure!(
        before["workspace_name"] != "keek",
        "Start this test from another workspace"
    );
    let mut fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("viewer").await?;
        let session = fixture.open("viewer", &worktree).await?;
        let after = host_state().await?;
        anyhow::ensure!(
            before == after,
            "open_session changed host focus or workspace"
        );
        let folder = fixture
            .directory
            .path()
            .join("cache/keek/sessions")
            .join(&session);
        let clients: Value =
            serde_json::from_str(&run("hyprctl", &["clients", "-j"], None).await?)?;
        let client = clients
            .as_array()
            .context("clients is not an array")?
            .iter()
            .find(|client| {
                if client["class"] != "aquamarine" {
                    return false;
                }
                let Some(pid) = client["pid"].as_u64() else {
                    return false;
                };
                std::fs::read(format!("/proc/{pid}/cmdline"))
                    .ok()
                    .is_some_and(|cmdline| {
                        cmdline
                            .split(|byte| *byte == 0)
                            .any(|arg| arg == folder.as_os_str().as_encoded_bytes())
                    })
            })
            .context("the session's aquamarine window was not found")?;
        let pid = client["pid"].as_u64().context("window has no pid")?;
        use keek::compositor::Compositor;
        let compositor =
            keek::compositor::HostCompositor::new(std::sync::Arc::new(keek::process::HostRunner));
        let placement = compositor
            .workspace(keek::config::Config::default().session_cap)
            .await?;
        let focus = placement.layout.focus;
        let slot =
            keek::slots::geometry(placement.layout.alone, Some((focus.width, focus.height))).window;
        anyhow::ensure!(
            client["workspace"]["name"] == "keek" && client["floating"] == true,
            "Session Window is not floating on keek: workspace {}, floating {}",
            client["workspace"]["name"],
            client["floating"]
        );
        anyhow::ensure!(
            client["at"] == json!([slot.x + placement.origin.0, slot.y + placement.origin.1]),
            "Session Window is at {} but its slot is at [{}, {}]",
            client["at"],
            slot.x + placement.origin.0,
            slot.y + placement.origin.1
        );
        anyhow::ensure!(
            client["size"] == json!([slot.width, slot.height]),
            "Session Window is {} but its slot is [{}, {}]",
            client["size"],
            slot.width,
            slot.height
        );
        let mut pending = vec![pid];
        let mut nested = None;
        while let Some(parent) = pending.pop() {
            if std::fs::read_to_string(format!("/proc/{parent}/comm"))
                .is_ok_and(|name| name.trim() == "Hyprland")
            {
                nested = Some(parent);
                break;
            }
            if let Ok(children) =
                std::fs::read_to_string(format!("/proc/{parent}/task/{parent}/children"))
            {
                pending.extend(
                    children
                        .split_whitespace()
                        .filter_map(|child| child.parse::<u64>().ok()),
                );
            }
        }
        let nested = nested.context("nested Hyprland process not found")?;
        let uid = unsafe { libc::getuid() };
        let runtime = format!("/proc/{nested}/root/run/user/{uid}/hypr");
        let socket = std::fs::read_dir(runtime)?
            .filter_map(Result::ok)
            .map(|entry| entry.path().join(".socket.sock"))
            .find(|path| path.exists())
            .context("nested Hyprland socket not found")?;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // A socket address is capped at 108 bytes, and the /proc path is longer.
        let link = fixture.directory.path().join("nested.sock");
        std::os::unix::fs::symlink(&socket, &link)?;
        let mut stream = tokio::net::UnixStream::connect(link).await?;
        stream.write_all(b"j/monitors").await?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await?;
        let monitors: Value = serde_json::from_str(&reply)?;
        anyhow::ensure!(
            monitors.as_array().is_some_and(|monitors| monitors
                .iter()
                .any(|monitor| monitor["width"] == slot.width && monitor["height"] == slot.height)),
            "nested output size differs from its slot"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result?;
    cleanup
}
