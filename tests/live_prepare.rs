#![allow(
    dead_code,
    reason = "The shared desktop fixture also supports recording tests."
)]
mod common;
include!("common/live.rs");

#[tokio::test]
#[ignore = "requires real bwrap, pasta, /usr/bin/mise and network access to download node"]
async fn live_prepare_project_installs_a_pinned_node_once_and_reaches_start() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture.worktree("node-project").await?;
        write_profile(
            fixture.directory.path(),
            "node-project",
            "start = \"node -e 'console.log(process.version); setInterval(() => {}, 1000)'\"\n[toolchain]\nnode = \"22\"\n",
        )
        .await?;
        let arguments = json!({"project": "node-project", "worktree": worktree});
        let first = call(&fixture.client, "prepare_project", arguments.clone()).await?;
        let second = call(&fixture.client, "prepare_project", arguments).await?;
        println!("first: {first:#}\nsecond: {second:#}");
        let node = &first["toolchains"][0];
        anyhow::ensure!(
            node["tool"] == "node" && node["version"].as_str().is_some_and(|v| v.starts_with("22.")),
            "node 22 was not resolved: {first}"
        );
        anyhow::ensure!(node["fetch"] == "installed", "the first run did not install node: {first}");
        anyhow::ensure!(
            second["toolchains"][0]["fetch"] == "present" && second["layer"] == "reused",
            "the second run downloaded or rebuilt: {second}"
        );
        for reply in [&first, &second] {
            anyhow::ensure!(
                reply["start"]["state"] == "running"
                    && reply["start_log"].as_str().is_some_and(|log| log.contains("v22.")),
                "start did not run node 22: {reply}"
            );
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}
