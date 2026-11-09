#![allow(
    dead_code,
    reason = "The shared desktop fixture also supports recording tests."
)]
mod common;
include!("common/live.rs");

#[tokio::test]
#[ignore = "requires real bwrap, pasta and a Hyprland desktop"]
async fn live_prepare_project_reads_test_files_and_refuses_a_write() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("test-files").await?;
        write_profile(
            fixture.directory.path(),
            "test-files",
            "start = \"touch /keek/files/probe; ls /keek/files\"\n",
        )
        .await?;
        let folder = fixture
            .directory
            .path()
            .join("config/keek/projects/test-files.files");
        let input = folder.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::create_dir_all(&input)?;
            std::fs::write(input.join("upload.pdf"), "sample input")
        })
        .await??;
        let reply = call(
            &fixture.client,
            "prepare_project",
            json!({"project":"test-files","worktree":worktree}),
        )
        .await?;
        let log = reply["start_log"]
            .as_str()
            .context("prepare_project returned no start_log")?;
        anyhow::ensure!(
            log.contains("Read-only file system"),
            "write was not refused: {reply}"
        );
        anyhow::ensure!(
            log.contains("upload.pdf"),
            "Test Files were not readable: {reply}"
        );
        anyhow::ensure!(!folder.join("probe").exists(), "start wrote a Test File");
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}

const UPLOAD_START: &str = r#"chromium --ozone-platform=wayland --kiosk --no-first-run --no-default-browser-check --password-store=basic --user-data-dir=$HOME/chromium 'data:text/html,<html><title>keek fixture</title><body style="margin:0"><label style="display:block;height:100vh">choose a file<input type="file" onchange="document.title=%27keek fixture picked %27+this.files[0].name+%27 %27+this.files[0].size"></label></body></html>'"#;

async fn titled_windows(client: &Client, session: &str) -> Result<Vec<(String, u64, u64)>> {
    let reply = call(client, "list_windows", json!({"session":session})).await?;
    Ok(reply["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|window| {
            Some((
                window["title"].as_str()?.to_owned(),
                window["pid"].as_u64()?,
                window["window_id"].as_u64()?,
            ))
        })
        .collect())
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver and Chromium"]
async fn live_chromium_uploads_a_test_file_through_its_file_dialog() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree_with_start("upload", UPLOAD_START).await?;
        let folder = fixture
            .directory
            .path()
            .join("config/keek/projects/upload.files");
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::create_dir_all(&folder)?;
            std::fs::write(folder.join("sample-plan.pdf"), "sample input")
        })
        .await??;
        let session = fixture.open("upload", &worktree).await?;
        fixture.click(&session, 1).await?;
        let mut dialog = None;
        let mut seen = Vec::new();
        for _ in 0..50 {
            seen = titled_windows(&fixture.client, &session).await?;
            dialog = seen
                .iter()
                .find(|(title, _, _)| !title.contains("keek fixture"))
                .cloned();
            if dialog.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        println!("windows after the click: {seen:?}");
        let (_, pid, window_id) = dialog.context("the file dialog never opened")?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let state = call(
            &fixture.client,
            "get_window_state",
            json!({"session":session,"pid":pid,"window_id":window_id}),
        )
        .await;
        println!("dialog get_window_state ok: {}", state.is_ok());
        call(
            &fixture.client,
            "hotkey",
            json!({"session":session,"pid":pid,"keys":["ctrl","l"]}),
        )
        .await?;
        call(
            &fixture.client,
            "type_text",
            json!({"session":session,"pid":pid,"text":"/keek/files/sample-plan.pdf"}),
        )
        .await?;
        call(
            &fixture.client,
            "press_key",
            json!({"session":session,"pid":pid,"key":"Return"}),
        )
        .await?;
        let mut picked = false;
        for _ in 0..50 {
            seen = titled_windows(&fixture.client, &session).await?;
            picked = seen
                .iter()
                .any(|(title, _, _)| title.contains("picked sample-plan.pdf 12"));
            if picked {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        println!("windows after the upload: {seen:?}");
        anyhow::ensure!(
            picked,
            "the page never reported the picked Test File: {seen:?}"
        );
        let records = fixture.close(&session).await?;
        let report = records.report.clone();
        let report = tokio::task::spawn_blocking(move || std::fs::read_to_string(report)).await??;
        println!("Verdict Report:\n{report}");
        anyhow::ensure!(
            report.contains("sample-plan.pdf"),
            "the Verdict Report does not list the Test File"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}
