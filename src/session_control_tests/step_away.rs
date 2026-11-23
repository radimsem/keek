use super::*;
use crate::ipc::TakeoverPhase;

const FULL_BINDS: &str =
    r#"[{"submap":"keek","key":"Escape","modmask":64},{"submap":"keek","key":"Tab","modmask":64}]"#;
const ESCAPE_ONLY: &str = r#"[{"submap":"keek","key":"Escape","modmask":64}]"#;
const PAUSED_SUMMARY: &str = "Takeover paused · app";

struct HostRunner {
    calls: Arc<StdMutex<Vec<String>>>,
    binds: &'static str,
}
impl ProcessRunner for HostRunner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            Ok(match args.first().copied() {
                Some("clients") => r#"[{"class":"editor","address":"0xdef","focusHistoryID":1},{"class":"aquamarine","address":"0xabc","focusHistoryID":0}]"#,
                Some("activewindow") => r#"{"address":"0xdef"}"#,
                Some("binds") => self.binds,
                _ => "ok",
            }
            .into())
        })
    }
}

fn host(
    binds: &'static str,
    host_events: Option<mpsc::Receiver<crate::hypr::HyprEvent>>,
) -> Fixture {
    let (control, receiver) = Control::new();
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let (events, snapshots) = watch::channel(SessionSnapshot {
        control: ControlState::Verifier,
        state: crate::ipc::SessionState::Open,
        checks: vec![],
        reminded: false,
        last_action: None,
    });
    let controller = Controller {
        control: control.clone(),
        runner: Arc::new(HostRunner {
            calls: calls.clone(),
            binds,
        }),
        compositor: Arc::new(FakeCompositor(StdMutex::new(host_events))),
        clock: Arc::new(crate::clock::MonotonicClock),
        events,
        opened: Instant::now(),
        window: Some(WindowAddress("0xabc".into())),
        project: "app".into(),
    };
    (
        control,
        tokio::spawn(controller.run(receiver, Box::pin(std::future::pending()))),
        calls,
        snapshots,
    )
}

async fn phase(snapshots: &mut watch::Receiver<SessionSnapshot>, paused: bool) {
    loop {
        if let ControlState::Takeover { phase, .. } = &snapshots.borrow_and_update().control
            && matches!(phase, TakeoverPhase::Paused { .. }) == paused
        {
            return;
        }
        snapshots.changed().await.unwrap();
    }
}

fn since(calls: &Arc<StdMutex<Vec<String>>>, from: usize) -> Vec<String> {
    calls.lock().unwrap()[from..].to_vec()
}

fn position(calls: &[String], needle: &str) -> usize {
    calls
        .iter()
        .position(|call| call.contains(needle))
        .unwrap_or_else(|| panic!("{needle} is missing from {calls:?}"))
}

fn held(control: &Control) -> tokio::task::JoinHandle<crate::session::gate::Entry> {
    let gate = control.gate.clone();
    tokio::spawn(async move { gate.enter().await })
}

#[tokio::test]
async fn step_away_gives_the_host_back_and_keeps_the_verifiers_call_held() {
    let (control, task, calls, mut snapshots) = host(FULL_BINDS, None);
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    assert!(matches!(
        snapshots.borrow().control,
        ControlState::Takeover {
            shortcuts: HostShortcuts::Captured,
            phase: TakeoverPhase::Running,
            ..
        }
    ));
    let call = held(&control);
    let before = calls.lock().unwrap().len();
    control.commands.send(Command::StepAway).await.unwrap();
    phase(&mut snapshots, true).await;
    let away = since(&calls, before);
    let reset = position(&away, "submap(\"reset\")");
    let no_focus = position(&away, "prop = \"no_focus\", value = \"1\"");
    let focus = position(&away, "hl.dsp.focus({ window = \"address:0xdef\" })");
    assert!(reset < no_focus && no_focus < focus, "{away:?}");
    assert!(
        away.iter()
            .any(|call| call.contains("inactive_border_color") && call.contains("rgb(")),
        "the paused card needs its yellow border: {away:?}"
    );
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        !call.is_finished(),
        "Step away released the Verifier's call"
    );
    assert!(control.drain().await.is_empty());
    call.abort();
    control.stop().await;
    task.await.unwrap();
}

#[tokio::test]
async fn resume_takes_control_again_within_the_same_takeover() {
    let (control, task, calls, mut snapshots) = host(FULL_BINDS, None);
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    let ControlState::Takeover { start_ms, .. } = snapshots.borrow().control.clone() else {
        panic!("expected a Takeover");
    };
    let call = held(&control);
    control.commands.send(Command::StepAway).await.unwrap();
    phase(&mut snapshots, true).await;
    let before = calls.lock().unwrap().len();
    control.commands.send(Command::Resume).await.unwrap();
    phase(&mut snapshots, false).await;
    let back = since(&calls, before);
    let border = position(&back, "prop = \"inactive_border_color\", value = \"unset\"");
    let no_focus = position(&back, "prop = \"no_focus\", value = \"0\"");
    let focus = position(&back, "hl.dsp.focus({ window = \"address:0xabc\" })");
    let submap = position(&back, "submap(\"keek\")");
    assert!(
        border < no_focus && no_focus < focus && focus < submap,
        "{back:?}"
    );
    assert!(matches!(
        snapshots.borrow().control,
        ControlState::Takeover { start_ms: resumed, .. } if resumed == start_ms
    ));
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(!call.is_finished(), "Resume released the Verifier's call");
    call.abort();
    control.stop().await;
    task.await.unwrap();
}

#[tokio::test]
async fn done_from_a_pause_releases_the_held_call_with_each_paused_span() {
    let (control, task, calls, mut snapshots) = host(FULL_BINDS, None);
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    let call = held(&control);
    control.commands.send(Command::StepAway).await.unwrap();
    phase(&mut snapshots, true).await;
    control.commands.send(Command::Resume).await.unwrap();
    phase(&mut snapshots, false).await;
    control.commands.send(Command::StepAway).await.unwrap();
    phase(&mut snapshots, true).await;
    let before = calls.lock().unwrap().len();
    control.commands.send(Command::Done).await.unwrap();
    let crate::session::gate::Entry::HandBack(span) = call.await.unwrap() else {
        panic!("Done from a pause did not hand back");
    };
    assert_eq!(span.paused.len(), 2, "{span:?}");
    assert!(span.paused.iter().all(|pause| span.start <= pause.start
        && pause.start <= pause.end
        && pause.end <= span.end));
    assert!(span.paused[0].end <= span.paused[1].start);
    let done = since(&calls, before);
    assert!(
        done.iter().all(|call| !call.contains("hl.dsp.focus")),
        "Done from a pause moved the developer's focus: {done:?}"
    );
    assert!(
        done.iter().all(|call| !call.contains("submap(")),
        "{done:?}"
    );
    position(&done, "prop = \"inactive_border_color\", value = \"unset\"");
    assert!(matches!(
        snapshots.borrow().control,
        ControlState::HandBack { .. }
    ));
    control.stop().await;
    task.await.unwrap();
    assert_eq!(control.drain().await.len(), 1);
}

#[tokio::test]
async fn the_paused_notification_goes_up_on_each_step_away_and_down_on_resume_and_done() {
    let (control, task, calls, mut snapshots) = host(FULL_BINDS, None);
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    control.commands.send(Command::StepAway).await.unwrap();
    phase(&mut snapshots, true).await;
    control.commands.send(Command::Resume).await.unwrap();
    phase(&mut snapshots, false).await;
    control.commands.send(Command::StepAway).await.unwrap();
    phase(&mut snapshots, true).await;
    control.commands.send(Command::Done).await.unwrap();
    control.stop().await;
    task.await.unwrap();
    let notices: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with("omarchy-notification"))
        .cloned()
        .collect();
    assert_eq!(notices.len(), 4, "{notices:?}");
    for (notice, program) in notices.iter().zip([
        "omarchy-notification-send ",
        "omarchy-notification-dismiss ",
        "omarchy-notification-send ",
        "omarchy-notification-dismiss ",
    ]) {
        assert!(
            notice.starts_with(program) && notice.contains(PAUSED_SUMMARY),
            "{notices:?}"
        );
    }
}

#[tokio::test]
async fn super_tab_steps_away_and_the_submap_reset_after_it_is_not_done() {
    let (sender, receiver) = mpsc::channel(4);
    let (control, task, _, mut snapshots) = host(FULL_BINDS, Some(receiver));
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    sender
        .send(crate::hypr::HyprEvent::Submap("keek".into()))
        .await
        .unwrap();
    sender
        .send(crate::hypr::HyprEvent::Submap("keek-away".into()))
        .await
        .unwrap();
    phase(&mut snapshots, true).await;
    // Hyprland reports keek's own reset of the submap after Step away.
    let _ = sender
        .send(crate::hypr::HyprEvent::Submap(String::new()))
        .await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        snapshots.borrow().control,
        ControlState::Takeover {
            phase: TakeoverPhase::Paused { .. },
            ..
        }
    ));
    assert!(control.drain().await.is_empty());
    control.stop().await;
    task.await.unwrap();
}

#[test]
fn step_away_detection_requires_super_tab_in_the_keek_submap() {
    for (submap, key, mask, expected) in [
        ("keek", "Tab", 64, true),
        ("keek", "tab", 65, true),
        ("other", "Tab", 64, false),
        ("keek", "Tab", 0, false),
        ("keek", "Escape", 64, false),
    ] {
        assert_eq!(
            has_step_away(&serde_json::json!([{"submap":submap,"key":key,"modmask":mask}])),
            expected,
            "{submap} {key} {mask}"
        );
    }
}

#[tokio::test]
async fn a_snippet_without_super_tab_is_reported_and_the_button_still_steps_away() {
    let (control, task, calls, mut snapshots) = host(ESCAPE_ONLY, None);
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    assert!(matches!(
        snapshots.borrow().control,
        ControlState::Takeover {
            shortcuts: HostShortcuts::CapturedWithoutStepAway,
            ..
        }
    ));
    let before = calls.lock().unwrap().len();
    control.commands.send(Command::StepAway).await.unwrap();
    phase(&mut snapshots, true).await;
    position(&since(&calls, before), "submap(\"reset\")");
    control.stop().await;
    task.await.unwrap();
}

#[tokio::test]
async fn ending_the_session_while_paused_finishes_the_takeover_and_leaves_focus_alone() {
    let (control, task, calls, mut snapshots) = host(FULL_BINDS, None);
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    control.commands.send(Command::StepAway).await.unwrap();
    phase(&mut snapshots, true).await;
    let before = calls.lock().unwrap().len();
    control.stop().await;
    task.await.unwrap();
    let spans = control.drain().await;
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].paused.len(), 1);
    let ended = since(&calls, before);
    assert!(
        ended
            .iter()
            .all(|call| !call.contains("hl.dsp.focus") && !call.contains("submap(")),
        "{ended:?}"
    );
    position(&ended, "omarchy-notification-dismiss");
    position(
        &ended,
        "prop = \"inactive_border_color\", value = \"unset\"",
    );
}

#[tokio::test]
async fn step_away_and_resume_outside_their_state_change_nothing() {
    let (control, task, calls, mut snapshots) = host(FULL_BINDS, None);
    control.commands.send(Command::StepAway).await.unwrap();
    control.commands.send(Command::Resume).await.unwrap();
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    let before = calls.lock().unwrap().len();
    control.commands.send(Command::Resume).await.unwrap();
    control.stop().await;
    task.await.unwrap();
    let calls = calls.lock().unwrap();
    assert!(
        calls[before..]
            .iter()
            .all(|call| !call.contains("prop = \"no_focus\", value = \"0\""))
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.contains("submap(\"keek\")"))
            .count(),
        1,
        "{calls:?}"
    );
}
