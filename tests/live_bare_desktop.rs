#![allow(
    dead_code,
    reason = "The shared desktop fixture also supports recording tests."
)]
mod common;
include!("common/live.rs");

fn toggles_listing(path: &Path) -> Result<Vec<std::ffi::OsString>> {
    let listing = match std::fs::read_dir(path) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut entries = listing
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver, Chromium and ffmpeg"]
async fn live_session_shows_only_the_app() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let toggles = PathBuf::from(std::env::var_os("HOME").context("HOME is missing")?)
        .join(".local/state/omarchy/toggles");
    let before = toggles_listing(&toggles)?;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let start = "(sleep 6; notify-send -u critical 'keek test') & exec chromium --ozone-platform=wayland --no-first-run --no-default-browser-check --password-store=basic --user-data-dir=$HOME/chromium 'data:text/html,<title>keek fixture</title><body>fixture<script>function t(){document.title=%27keek fixture %27+innerWidth+%27x%27+innerHeight}t();onresize=t</script>'";
        let worktree = fixture.worktree_with_start("bare", start).await?;
        write_profile(fixture.directory.path(), "bare", &format!("start = {}\n[window]\nwidth = 1440\nheight = 960\n", toml::Value::String(start.to_owned()))).await?;
        let session = fixture.open("bare", &worktree).await?;
        fixture.click(&session, 0).await?;
        // A notification sent as `start`'s first command showed in no frame, so `start` sends it 6 s in.
        tokio::time::sleep(Duration::from_secs(8)).await;
        let windows = call(&fixture.client, "list_windows", json!({"session":session})).await?;
        let window = windows["windows"].as_array().context("no windows")?.iter().find(|window| window["title"].as_str().is_some_and(|title| title.contains("keek fixture"))).context("no fixture window")?;
        println!("list_windows: {window}");
        anyhow::ensure!(
            window["x"] == 0 && window["y"] == 0 && window["width"] == 1440 && window["height"] == 960,
            "the window is not 1440x960 at 0,0: {window}"
        );
        let records = fixture.close(&session).await?;
        // The fixture's folder is removed when the test ends, so the footage is kept under target/tmp.
        let kept = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let raw = kept.join("bare-desktop-raw.mp4");
        let (from, to) = (records.raw.clone(), raw.clone());
        tokio::task::spawn_blocking(move || std::fs::copy(from, to)).await??;
        let frame = kept.join("bare-desktop-frame.png");
        let status = Command::new("ffmpeg").args(["-y", "-sseof", "-1", "-i"]).arg(&raw).args(["-frames:v", "1", "-update", "1"]).arg(&frame).status().await?;
        anyhow::ensure!(status.success(), "ffmpeg could not extract a frame");
        println!("Inspect for the bar, gaps and the notification: {} and {}", frame.display(), raw.display());

        let kiosk = "exec chromium --ozone-platform=wayland --kiosk --no-first-run --no-default-browser-check --password-store=basic --user-data-dir=$HOME/chromium --app='data:text/html,<script>function t(){document.title=innerWidth+%27x%27+innerHeight}t();onresize=t</script>'";
        let worktree = fixture.worktree_with_start("kiosk", kiosk).await?;
        write_profile(fixture.directory.path(), "kiosk", &format!("start = {}\n[window]\nwidth = 1440\nheight = 960\n", toml::Value::String(kiosk.to_owned()))).await?;
        let session = fixture.open("kiosk", &worktree).await?;
        let mut titles = Vec::new();
        for _ in 0..150 {
            let windows = call(&fixture.client, "list_windows", json!({"session":session})).await?;
            titles = windows["windows"].as_array().into_iter().flatten().filter_map(|window| window["title"].as_str().map(str::to_owned)).collect();
            if titles.iter().any(|title| title == "1440x960") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        println!("kiosk titles: {titles:?}");
        anyhow::ensure!(titles.iter().any(|title| title == "1440x960"), "the kiosk page did not report 1440x960; its windows were titled {titles:?}");
        fixture.close(&session).await?;
        Ok::<(), anyhow::Error>(())
    }.await;
    let cleanup = fixture.finish().await;
    let after = toggles_listing(&toggles)?;
    assert_eq!(before, after);
    result?;
    cleanup
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver, Chromium and ffmpeg"]
async fn live_the_first_recorded_frame_shows_the_app_instead_of_a_black_cover() -> Result<()> {
    async fn first_frame(path: &Path) -> Result<Vec<u8>> {
        let output = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=32:32",
                "-pix_fmt",
                "gray",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "first frame extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        anyhow::ensure!(
            output.stdout.len() == 32 * 32,
            "missing or incomplete first frame: {} bytes",
            output.stdout.len()
        );
        Ok(output.stdout)
    }

    let _desktop = LIVE_DESKTOP.lock().await;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let control = fixture.directory.path().join("white.mp4");
        let output = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=32x32",
                "-frames:v",
                "1",
            ])
            .arg(&control)
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "white control creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let white = first_frame(&control).await?;
        anyhow::ensure!(
            white.iter().map(|&pixel| u64::from(pixel)).sum::<u64>() / white.len() as u64 > 200,
            "white control is not bright"
        );

        let worktree = fixture
            .worktree_with_start("first-frame", START_COMMAND)
            .await?;
        let session = fixture.open("first-frame", &worktree).await?;
        let records = fixture.close(&session).await?;
        let frame = first_frame(&records.raw).await?;
        let mean =
            frame.iter().map(|&pixel| u64::from(pixel)).sum::<u64>() as f64 / frame.len() as f64;
        anyhow::ensure!(
            mean > 64.0,
            "the first Raw Recording frame is black: mean luma {mean}"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result?;
    cleanup
}
