#![allow(
    dead_code,
    reason = "The shared desktop fixture supports other live tests."
)]
include!("common/live.rs");
include!("common/live_evidence.rs");
mod common;

const BAR_WAIT: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore = "requires real Hyprland, bwrap, pasta, Chromium and pinned Cua Driver"]
async fn live_bar_state_follows_two_sessions_and_a_help_request() -> Result<()> {
    let _desktop = LIVE_DESKTOP.lock().await;
    let before = (host_state().await?, host_config().await?);
    let mut fixture = Fixture::start().await?;
    let bar = fixture.directory.path().join("run/keek/bar.json");
    let result = follow_two_sessions(&mut fixture, &bar).await;
    let finished = fixture.finish().await;
    result.and(finished)?;
    anyhow::ensure!(
        !bar.exists(),
        "bar.json remains after the daemon stopped: {}",
        bar.display()
    );
    println!("bar.json: removed once the daemon stopped");
    let after = (host_state().await?, host_config().await?);
    anyhow::ensure!(
        before == after,
        "host focus, workspace or Hyprland config changed: before={:?}, after={:?}",
        before.0,
        after.0
    );
    Ok(())
}

async fn follow_two_sessions(fixture: &mut Fixture, bar: &Path) -> Result<()> {
    let first_tree = fixture.worktree("bar-first").await?;
    let first = fixture.open("bar-first", &first_tree).await?;
    let second_tree = fixture.worktree("bar-second").await?;
    let second = fixture.open("bar-second", &second_tree).await?;

    let listed = wait_for_bar("both sessions to be listed as working", bar, |state| {
        session_state(state, 1) == Some("working") && session_state(state, 2) == Some("working")
    })
    .await?;
    anyhow::ensure!(listed["version"] == 1, "bar.json version differs: {listed}");
    println!("bar.json: sessions 1 and 2 listed as working");

    let mut helping = Box::pin(call(
        &fixture.client,
        "request_help",
        json!({"session":second,"reason":"2FA code"}),
    ));
    let raised = tokio::select! {
        outcome = &mut helping => anyhow::bail!("request_help returned before anyone answered: {outcome:?}"),
        state = wait_for_bar("session 2 to need you", bar, |state| {
            session_state(state, 2) == Some("needs_you")
        }) => state?,
    };
    anyhow::ensure!(
        session_state(&raised, 1) == Some("working"),
        "session 1 left working: {raised}"
    );
    anyhow::ensure!(
        session_detail(&raised, 2) == Some("2FA code"),
        "session 2 does not show the reason: {raised}"
    );
    println!("bar.json: session 2 needs_you with its reason, session 1 still working");

    // Dropping the call sends the cancellation that withdraws the Help Request.
    drop(helping);
    wait_for_bar("session 2 to return to working", bar, |state| {
        session_state(state, 2) == Some("working")
    })
    .await?;
    println!("bar.json: session 2 back to working after the withdrawal");

    fixture.close(&first).await?;
    wait_for_bar("session 1 to end", bar, |state| {
        matches!(session_state(state, 1), None | Some("ended"))
            && session_state(state, 2) == Some("working")
    })
    .await?;
    println!("bar.json: session 1 ended or gone, session 2 still working");

    fixture.close(&second).await?;
    let outcome = wait_for_ended_or_gone(bar).await?;
    println!("bar.json after both sessions closed: {outcome}");
    Ok(())
}

fn session_field<'a>(state: &'a Value, n: u64, field: &str) -> Option<&'a str> {
    state
        .get("sessions")?
        .as_array()?
        .iter()
        .find(|session| session.get("n").and_then(Value::as_u64) == Some(n))?
        .get(field)?
        .as_str()
}

fn session_state(state: &Value, n: u64) -> Option<&str> {
    session_field(state, n, "state")
}

fn session_detail(state: &Value, n: u64) -> Option<&str> {
    session_field(state, n, "detail")
}

async fn wait_for_bar(what: &str, bar: &Path, accept: fn(&Value) -> bool) -> Result<Value> {
    let bar = bar.to_owned();
    wait_for(what, BAR_WAIT, move || {
        let state: Value = serde_json::from_str(&std::fs::read_to_string(&bar).ok()?).ok()?;
        accept(&state).then_some(state)
    })
    .await
}

/// An ended Session Card stays until the developer dismisses it, so bar.json can outlive close_session.
async fn wait_for_ended_or_gone(bar: &Path) -> Result<&'static str> {
    let bar = bar.to_owned();
    wait_for(
        "both sessions to end",
        BAR_WAIT,
        move || match std::fs::read_to_string(&bar) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some("file removed"),
            Err(_) => None,
            Ok(text) => {
                let state: Value = serde_json::from_str(&text).ok()?;
                let sessions = state.get("sessions")?.as_array()?;
                sessions
                    .iter()
                    .all(|session| session.get("state").and_then(Value::as_str) == Some("ended"))
                    .then_some("every listed session ended")
            }
        },
    )
    .await
}
