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
