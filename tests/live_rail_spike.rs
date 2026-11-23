//! Spike for #88. Measures how a real Agent Session behaves while its Session Window is shown and hidden,
//! and what a still of it costs. Nothing here ships: the results go to docs/facts.md.
#![allow(dead_code)]
mod common;
include!("common/live.rs");

use std::time::Instant;

fn out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/spike-rail")
}

async fn session_window() -> Result<Value> {
    let clients: Value = serde_json::from_str(&run("hyprctl", &["clients", "-j"], None).await?)?;
    let windows: Vec<Value> = clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|client| client["class"] == "aquamarine")
        .cloned()
        .collect();
    anyhow::ensure!(
        windows.len() == 1,
        "expected one aquamarine window, found {}",
        windows.len()
    );
    Ok(windows[0].clone())
}

async fn eval(lua: &str) -> Result<()> {
    let output = run("hyprctl", &["eval", lua], None).await?;
    anyhow::ensure!(output == "ok", "hyprctl eval refused: {output}");
    Ok(())
}

async fn move_window(address: &str, workspace: &str, x: i64, y: i64) -> Result<()> {
    eval(&format!(
        "hl.dispatch(hl.dsp.window.move({{ window = 'address:{address}', workspace = '{workspace}', follow = false }})); hl.dispatch(hl.dsp.window.move({{ window = 'address:{address}', x = {x}, y = {y}, relative = false }}))"
    ))
    .await
}

/// The developer's workspace must stay put and the Session Window must never take focus.
async fn guard(before: &Value, address: &str) -> Result<()> {
    let now = host_state().await?;
    // The developer may switch between their own windows while this runs.
    anyhow::ensure!(
        now["workspace_id"] == before["workspace_id"] && now["address"] != address,
        "the workspace moved or the Session Window took focus: before={before}, now={now}"
    );
    if &now != before {
        let active = run("hyprctl", &["activewindow", "-j"], None).await?;
        let active: Value = serde_json::from_str(&active)?;
        println!("focus is on {} ({})", active["class"], now["address"]);
    }
    let no_focus = run(
        "hyprctl",
        &["getprop", &format!("address:{address}"), "no_focus"],
        None,
    )
    .await?;
    anyhow::ensure!(no_focus == "true", "no_focus is {no_focus}");
    Ok(())
}

fn children_cpu_ms() -> f64 {
    // SAFETY: getrusage writes into a zeroed struct we own and has no other preconditions.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_CHILDREN, &mut usage);
        usage
    };
    let ms = |time: libc::timeval| time.tv_sec as f64 * 1000.0 + time.tv_usec as f64 / 1000.0;
    ms(usage.ru_utime) + ms(usage.ru_stime)
}

struct StillCost {
    wall_ms: f64,
    cpu_ms: f64,
    bytes: u64,
}

async fn still(stable_id: &str, file: &Path) -> Result<StillCost> {
    let cpu = children_cpu_ms();
    let start = Instant::now();
    let output = Command::new("grim")
        .args(["-T", stable_id, "-t", "jpeg", "-q", "70", "-s", "0.5"])
        .arg(file)
        .output()
        .await?;
    let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
    anyhow::ensure!(
        output.status.success(),
        "grim -T failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(StillCost {
        wall_ms,
        cpu_ms: children_cpu_ms() - cpu,
        bytes: std::fs::metadata(file)?.len(),
    })
}

/// The runtime folder of the sandbox that owns a Session Window, reached through /proc.
fn sandbox_runtime(root_pid: u32, uid: u32) -> Option<PathBuf> {
    let own = std::fs::read_link("/proc/self/ns/mnt").ok()?;
    let mut queue = vec![root_pid];
    let mut seen = 0;
    while let Some(pid) = queue.pop() {
        seen += 1;
        if seen > 400 {
            return None;
        }
        let namespace = std::fs::read_link(format!("/proc/{pid}/ns/mnt")).ok();
        if namespace.is_some_and(|namespace| namespace != own) {
            let runtime = PathBuf::from(format!("/proc/{pid}/root/run/user/{uid}"));
            let has_socket = std::fs::read_dir(&runtime).ok().is_some_and(|entries| {
                entries.flatten().any(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name.starts_with("wayland-") && !name.ends_with(".lock")
                })
            });
            if has_socket {
                return Some(runtime);
            }
        }
        if let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) {
            for task in tasks.flatten() {
                if let Ok(text) = std::fs::read_to_string(task.path().join("children")) {
                    queue.extend(
                        text.split_whitespace()
                            .filter_map(|child| child.parse::<u32>().ok()),
                    );
                }
            }
        }
    }
    None
}

async fn click(fixture: &Fixture, session: &str, pid: u64, window_id: u64) -> Result<f64> {
    let start = Instant::now();
    call(
        &fixture.client,
        "click",
        json!({"session":session,"pid":pid,"window_id":window_id,"x":195,"y":400}),
    )
    .await?;
    Ok(start.elapsed().as_secs_f64() * 1000.0)
}

fn stamp(start: &Instant, text: &str) {
    println!("[{:7.2}s] {text}", start.elapsed().as_secs_f64());
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver, Chromium, grim and ffprobe"]
async fn live_a_session_window_out_of_sight_keeps_recording_and_gives_stills() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let out = out_dir();
    std::fs::create_dir_all(&out)?;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("rail-spike").await?;
        let session = fixture.open("rail-spike", &worktree).await?;
        let begun = Instant::now();
        let window = session_window().await?;
        let address = window["address"].as_str().context("no address")?.to_owned();
        let stable = window["stableId"].as_str().context("no stable id")?.to_owned();
        let host_pid = window["pid"].as_u64().context("no pid")? as u32;
        let (home_x, home_y) = (window["at"][0].as_i64().unwrap_or(0), window["at"][1].as_i64().unwrap_or(0));
        let workspace = host_state().await?["workspace_id"].as_i64().context("no workspace")?.to_string();
        println!("session window {address} stable={stable} pid={host_pid} on {} at {home_x},{home_y} size {}", window["workspace"]["name"], window["size"]);
        guard(&before, &address).await?;

        // The Verifier's window, found as the live tests do.
        let (pid, window_id) = {
            let mut found = None;
            for _ in 0..150 {
                let listed = call(&fixture.client, "list_windows", json!({"session":&session})).await?;
                found = listed["windows"].as_array().and_then(|windows| {
                    windows.iter().find_map(|window| {
                        window["title"].as_str()?.contains("keek fixture").then_some(())?;
                        Some((window["pid"].as_u64()?, window["window_id"].as_u64()?))
                    })
                });
                if found.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            found.context("list_windows never found Chromium")?
        };
        tokio::time::sleep(Duration::from_secs(5)).await;
        call(&fixture.client, "get_window_state", json!({"session":&session,"pid":pid,"window_id":window_id})).await?;
        let mut clicks = 0usize;

        // Where the nested socket is, and whether the host can reach it.
        // SAFETY: getuid takes no pointers and has no preconditions.
        let uid = unsafe { libc::getuid() };
        match sandbox_runtime(host_pid, uid) {
            Some(runtime) => {
                let file = out.join("live-still-nested.jpg");
                let grim = Command::new("grim")
                    .args(["-t", "jpeg", "-q", "70", "-s", "0.5"])
                    .arg(&file)
                    .env("XDG_RUNTIME_DIR", &runtime)
                    .env("WAYLAND_DISPLAY", "wayland-1")
                    .output()
                    .await?;
                println!(
                    "nested socket at {}: grim exit {} {}",
                    runtime.display(),
                    grim.status,
                    String::from_utf8_lossy(&grim.stderr).trim()
                );
            }
            None => println!("no readable sandbox runtime folder under pid {host_pid} (the host cannot reach the nested socket through /proc)"),
        }

        // Hidden, no stills: the control.
        stamp(&begun, "PHASE hidden-control starts");
        let mut alone = Vec::new();
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_secs(4)).await;
            alone.push(click(&fixture, &session, pid, window_id).await?);
            clicks += 1;
        }
        println!("click latency alone, ms: {alone:.0?}");
        stamp(&begun, "PHASE hidden-stills starts");
        let mut costs = Vec::new();
        let mut together = Vec::new();
        for index in 0..8 {
            let file = out.join(format!("live-still-{index}.jpg"));
            if index % 2 == 1 {
                let (latency, cost) = tokio::join!(click(&fixture, &session, pid, window_id), still(&stable, &file));
                together.push(latency?);
                clicks += 1;
                costs.push(cost?);
            } else {
                costs.push(still(&stable, &file).await?);
            }
            guard(&before, &address).await?;
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        for (index, cost) in costs.iter().enumerate() {
            println!("still {index}: wall {:.0} ms, cpu {:.0} ms, {} bytes", cost.wall_ms, cost.cpu_ms, cost.bytes);
        }
        println!("click latency while a still is taken, ms: {together:.0?}");

        // On screen for a while.
        stamp(&begun, "PHASE shown starts");
        move_window(&address, &workspace, 2000, 40).await?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        guard(&before, &address).await?;
        let shown_cost = still(&stable, &out.join("live-still-shown.jpg")).await?;
        println!("still of a shown window: wall {:.0} ms, cpu {:.0} ms, {} bytes", shown_cost.wall_ms, shown_cost.cpu_ms, shown_cost.bytes);
        let shown_click = click(&fixture, &session, pid, window_id).await?;
        clicks += 1;
        println!("click latency while shown, ms: {shown_click:.0}");
        tokio::time::sleep(Duration::from_secs(4)).await;
        move_window(&address, "name:keek", home_x, home_y).await?;
        tokio::time::sleep(Duration::from_secs(4)).await;
        guard(&before, &address).await?;

        // Ten swaps between shown and hidden, with a Verifier click in the middle of most hidden spans.
        stamp(&begun, "PHASE swaps start");
        for round in 1..=10 {
            let started = Instant::now();
            move_window(&address, &workspace, 2000, 40).await?;
            stamp(&begun, &format!("swap {round} shown ({} ms)", started.elapsed().as_millis()));
            guard(&before, &address).await?;
            tokio::time::sleep(Duration::from_millis(1200)).await;
            let started = Instant::now();
            move_window(&address, "name:keek", home_x, home_y).await?;
            stamp(&begun, &format!("swap {round} hidden ({} ms)", started.elapsed().as_millis()));
            guard(&before, &address).await?;
            tokio::time::sleep(Duration::from_millis(900)).await;
            if round % 2 == 0 {
                click(&fixture, &session, pid, window_id).await?;
                clicks += 1;
            }
            tokio::time::sleep(Duration::from_millis(900)).await;
        }
        stamp(&begun, "PHASE swaps end");
        let end = session_window().await?;
        println!("session window after the swaps: {}", json!({"workspace":end["workspace"]["name"],"at":end["at"],"size":end["size"],"floating":end["floating"]}));
        tokio::time::sleep(Duration::from_secs(3)).await;

        let records = fixture.close(&session).await?;
        println!("clicks made: {clicks}, stills taken: {}", costs.len() + 1);
        std::fs::copy(&records.raw, out.join("live-raw.mp4"))?;
        std::fs::copy(&records.timeline, out.join("live-timeline.json"))?;
        let mut turns: Vec<_> = std::fs::read_dir(&records.actions)?.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
        turns.sort();
        println!("actions/ holds {} entries: {turns:?}", turns.len());
        assert_records(&records, clicks, pid).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if result.is_err()
        && let Ok(window) = session_window().await
        && let Some(address) = window["address"].as_str()
    {
        // A failed swap must not leave the window on the developer's screen.
        let _ = move_window(address, "name:keek", 0, 0).await;
    }
    let cleanup = fixture.finish().await;
    let after = host_state().await?;
    println!("host state before={before}, after={after}");
    result?;
    anyhow::ensure!(
        before["workspace_id"] == after["workspace_id"],
        "host workspace changed"
    );
    cleanup
}
