#![allow(
    dead_code,
    reason = "The shared desktop fixture supports other live tests."
)]
mod common;
include!("common/live.rs");

#[tokio::test]
#[ignore = "requires the real Hyprland desktop, pinned Cua Driver and Chromium"]
async fn live_click_with_only_window_id_lists_the_windows() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("refusal-hint").await?;
        let session = fixture.open("refusal-hint", &worktree).await?;
        let mut window = None;
        for _ in 0..150 {
            let result = call(&fixture.client, "list_windows", json!({"session":session})).await?;
            window = result["windows"].as_array().and_then(|windows| {
                windows.iter().find_map(|window| {
                    window["title"]
                        .as_str()?
                        .contains("keek fixture")
                        .then_some((window["pid"].as_u64()?, window["window_id"].as_u64()?))
                })
            });
            if window.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let (pid, window_id) = window.context("list_windows never found Chromium")?;
        let reply = fixture
            .client
            .call_tool(
                CallToolRequestParams::new("click").with_arguments(
                    json!({"session":session,"window_id":window_id,"x":195,"y":400})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await?;
        let value = serde_json::to_value(reply)?;
        println!("{value}");
        let text = value["content"].to_string();
        anyhow::ensure!(
            text.contains("Missing required integer field: pid"),
            "expected missing pid refusal: {text}"
        );
        anyhow::ensure!(text.contains(&format!("pid={pid}")), "expected pid: {text}");
        anyhow::ensure!(
            text.contains(&format!("window_id={window_id}")),
            "expected window_id: {text}"
        );
        fixture.close(&session).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}
