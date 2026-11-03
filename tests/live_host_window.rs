#![allow(dead_code)]
mod common;
include!("common/live.rs");
include!("common/live_evidence.rs");

#[tokio::test]
#[ignore = "requires the real Hyprland desktop and pinned Cua Driver"]
async fn live_a_second_host_window_ends_the_session_as_incomplete() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = host_state().await?;
    let mut events = keek::hypr::subscribe().await?;
    let mut fixture = Fixture::start().await?;
    let result = async {
        let worktree = fixture
            .worktree_with_start(
                "host-window",
                "bash -c 'sleep 3; hyprctl output create wayland; sleep 60'",
            )
            .await?;
        let session = fixture.open("host-window", &worktree).await?;
        let gap = tokio::time::timeout(Duration::from_secs(30), async {
            let mut session_window = None;
            let mut stray = None;
            let mut opened = None;
            while let Some(line) = events.next_line().await? {
                if let Some(keek::hypr::HyprEvent::OpenWindow { address, class, .. }) =
                    keek::hypr::parse(&line)
                    && class == "aquamarine"
                {
                    if session_window.is_none() {
                        session_window = Some(address);
                    } else {
                        stray = Some(address);
                        opened = Some(tokio::time::Instant::now());
                    }
                }
                if let Some(address) = &stray {
                    anyhow::ensure!(
                        !line
                            .strip_prefix("activewindowv2>>")
                            .is_some_and(|focused| focused.trim_start_matches("0x") == address),
                        "stray host window received focus"
                    );
                    if let Some(keek::hypr::HyprEvent::CloseWindow(closed)) =
                        keek::hypr::parse(&line)
                        && &closed == address
                    {
                        return Ok::<_, anyhow::Error>(
                            opened.context("no open timestamp")?.elapsed(),
                        );
                    }
                }
            }
            bail!("host events ended before the stray window closed")
        })
        .await??;
        println!("stray openwindow to closewindow gap: {gap:?}");
        anyhow::ensure!(
            gap < Duration::from_secs(10),
            "sandbox termination took too long"
        );
        let (_, report) =
            incomplete_report(&fixture, "host-window", Duration::from_secs(120)).await?;
        anyhow::ensure!(report.contains(&session), "wrong session: {report}");
        anyhow::ensure!(
            report.contains("opened a window on the developer's desktop"),
            "wrong cause: {report}"
        );
        anyhow::ensure!(!report.contains("## Verdict"), "unexpected Verdict");
        assert_processes_gone(
            &fixture
                .directory
                .path()
                .join("cache/keek/sessions")
                .join(&session),
        )
        .await?;
        anyhow::ensure!(
            before == host_state().await?,
            "host focus or workspace changed"
        );
        fixture.sessions.retain(|name| name != &session);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let cleanup = fixture.finish().await;
    result.and(cleanup)
}
