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
        for key in ["report","raw","timeline","actions","index"] {
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
