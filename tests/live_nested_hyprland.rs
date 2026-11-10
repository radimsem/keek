#![allow(dead_code)]
mod common;
include!("common/live.rs");

// The developer's own config would autostart their bar and daemons a second time.
const BARE_CONFIG: &str = r#"hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })
hl.config({ xwayland = { enabled = false }, misc = { disable_watchdog_warning = true } })
"#;

async fn host_window(pid: u32) -> Result<Option<Value>> {
    let clients: Value = serde_json::from_str(&run("hyprctl", &["clients", "-j"], None).await?)?;
    let Some(client) = clients
        .as_array()
        .into_iter()
        .flatten()
        .find(|client| client["pid"] == pid)
    else {
        return Ok(None);
    };
    let address = client["address"]
        .as_str()
        .context("host window has no address")?;
    let no_focus = run(
        "hyprctl",
        &["getprop", &format!("address:{address}"), "no_focus"],
        None,
    )
    .await?;
    Ok(Some(json!({
        "workspace": client["workspace"]["name"],
        "floating": client["floating"],
        "size": client["size"],
        "no_focus": no_focus,
    })))
}

async fn other_window_owners(developer: u32) -> Result<Vec<String>> {
    let clients: Value = serde_json::from_str(&run("hyprctl", &["clients", "-j"], None).await?)?;
    let mut owners = Vec::new();
    for client in clients.as_array().into_iter().flatten() {
        if client["class"] == "aquamarine" && client["pid"] != developer {
            let pid = client["pid"].as_u64().context("host window has no pid")?;
            owners.push(run("ps", &["-o", "comm=", "-p", &pid.to_string()], None).await?);
        }
    }
    Ok(owners)
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver and Chromium"]
async fn live_a_developers_nested_hyprland_returns_when_the_last_session_closes() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    anyhow::ensure!(
        before["workspace_name"] != "keek",
        "keek leaves a caught window in place while the keek workspace is active. Switch to another workspace and run again"
    );
    let mut fixture = Fixture::start().await?;
    let mut nested = None;
    let result = async {
        let worktree = fixture.worktree("nested").await?;
        let session = fixture.open("nested", &worktree).await?;
        let config = fixture.directory.path().join("bare.lua");
        let runtime = fixture.directory.path().join("nested-run");
        let (written, created) = (config.clone(), runtime.clone());
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir(created)?;
            std::fs::write(written, BARE_CONFIG)
        })
        .await??;
        let child = nested.insert(
            Command::new("Hyprland")
                .arg("--config")
                .arg(&config)
                // Hyprland leaves its instance folder behind on exit, so it goes in the fixture's folder.
                .env("XDG_RUNTIME_DIR", &runtime)
                .env("WAYLAND_DISPLAY", host_display()?)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()?,
        );
        let pid = child.id().context("nested Hyprland has no pid")?;
        let caught = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(window) = host_window(pid).await? {
                    return Ok::<_, anyhow::Error>(window);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("the developer's nested Hyprland never opened a host window")??;
        println!("with a session open: {caught}");
        anyhow::ensure!(
            caught["workspace"] == "keek" && caught["floating"] == true,
            "the host rule did not catch the developer's window: {caught}"
        );
        let owners = other_window_owners(pid).await?;
        println!("Session Window owners: {owners:?}");
        anyhow::ensure!(
            !owners.is_empty() && owners.iter().all(|owner| owner != "Hyprland"),
            "keek tells a Session Window from the developer's by its owner's name: {owners:?}"
        );
        fixture.close(&session).await?;
        let released = host_window(pid)
            .await?
            .context("the developer's nested Hyprland closed with the session")?;
        println!("after the last session closed: {released}");
        anyhow::ensure!(
            released["no_focus"] == "false",
            "no_focus stayed: {released}"
        );
        anyhow::ensure!(
            released["workspace"] == before["workspace_name"],
            "the window stayed off the developer's workspace: {released}"
        );
        anyhow::ensure!(
            released["floating"] == false,
            "the window stayed floating: {released}"
        );
        anyhow::ensure!(
            before == host_state().await?,
            "host focus or workspace changed"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let stopped = match nested {
        Some(mut child) => child.kill().await.map_err(anyhow::Error::from),
        None => Ok(()),
    };
    let cleanup = fixture.finish().await;
    result.and(stopped).and(cleanup)
}
