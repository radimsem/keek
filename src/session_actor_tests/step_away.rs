use super::*;
use crate::ipc::{ControlState, TakeoverPhase};

async fn send(actor: &RunningActor, command: control::Command) {
    actor.control.commands.send(command).await.unwrap();
}

async fn reach(actor: &RunningActor, paused: Option<bool>) {
    let mut events = actor.events.clone();
    loop {
        let reached = match &events.borrow_and_update().control {
            ControlState::Takeover { phase, .. } => {
                paused.is_some_and(|paused| matches!(phase, TakeoverPhase::Paused { .. }) == paused)
            }
            ControlState::HandBack { .. } => paused.is_none(),
            ControlState::Verifier | ControlState::HelpRequest { .. } => false,
        };
        if reached {
            return;
        }
        events.changed().await.unwrap();
    }
}

fn count(actor: &RunningActor, tool: &str) -> usize {
    actor
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == tool)
        .count()
}

async fn wait_minutes(minutes: u32) {
    for _ in 0..minutes {
        tokio::time::sleep(Duration::from_secs(60)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_paused_takeover_holds_the_call_pauses_the_idle_timeout_and_keeps_recording() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "step-away-held", false).await;
    send(&actor, control::Command::TakeControl).await;
    reach(&actor, Some(false)).await;
    let held = tokio::spawn({
        let sender = actor.sender.clone();
        let gate = actor.control.gate.clone();
        async move { super::takeover::forward(&sender, &gate, "click").await }
    });
    send(&actor, control::Command::StepAway).await;
    reach(&actor, Some(true)).await;
    let before = count(&actor, "get_recording_state");
    // Longer than the 5-minute idle timeout.
    wait_minutes(7).await;
    assert!(
        !held.is_finished(),
        "Step away released the Verifier's call"
    );
    assert!(
        !actor.task.is_finished(),
        "the idle timeout ended a paused Takeover"
    );
    let kept = count(&actor, "get_recording_state") - before;
    assert!(kept >= 6, "{kept} keepalives in 7 minutes away");
    send(&actor, control::Command::Resume).await;
    reach(&actor, Some(false)).await;
    assert!(!held.is_finished(), "Resume released the Verifier's call");
    send(&actor, control::Command::Done).await;
    let Response::ToolResult(value) = held.await.unwrap() else {
        panic!("expected the hand-back");
    };
    assert!(
        value["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("The developer pressed Done")),
        "{value}"
    );
    assert_eq!(count(&actor, "click"), 0);
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn resume_keeps_the_take_control_screen_as_the_baseline() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "step-away-baseline", false).await;
    let image = |block: bool| {
        let value = super::super::screen::fake_screenshot(200, 100, |pixbuf| {
            pixbuf.fill(0x000000ff);
            if block {
                pixbuf.new_subpixbuf(0, 0, 20, 10).fill(0xffffffff);
            }
        });
        fake_driver::Screenshot::Image(
            "image/png",
            Box::leak(
                value["content"][0]["data"]
                    .as_str()
                    .unwrap()
                    .to_owned()
                    .into_boxed_str(),
            ),
        )
    };
    let previous = super::screen_hand_back::captures(&actor);
    *actor.screenshot.lock().unwrap() = image(false);
    send(&actor, control::Command::TakeControl).await;
    reach(&actor, Some(false)).await;
    super::screen_hand_back::wait_for_baseline(&actor, previous).await;
    let held = tokio::spawn({
        let sender = actor.sender.clone();
        let gate = actor.control.gate.clone();
        async move { super::takeover::forward(&sender, &gate, "click").await }
    });
    send(&actor, control::Command::StepAway).await;
    reach(&actor, Some(true)).await;
    *actor.screenshot.lock().unwrap() = image(true);
    send(&actor, control::Command::Resume).await;
    reach(&actor, Some(false)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    send(&actor, control::Command::Done).await;
    let Response::ToolResult(value) = held.await.unwrap() else {
        panic!("expected the hand-back");
    };
    assert!(
        value["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("The screen may have changed.")),
        "Resume replaced the Take control screen: {value}"
    );
    assert_eq!(super::screen_hand_back::captures(&actor), previous + 2);
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_timeline_keeps_one_takeover_with_each_paused_span_inside_it() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "step-away-timeline", false).await;
    let tick = |ms| actor.clock.elapsed.fetch_add(ms, Ordering::SeqCst);
    // Past the video lead, so the Timeline does not clamp the Takeover start to zero.
    tick(10_000);
    send(&actor, control::Command::TakeControl).await;
    reach(&actor, Some(false)).await;
    tick(1_000);
    send(&actor, control::Command::StepAway).await;
    reach(&actor, Some(true)).await;
    tick(5_000);
    send(&actor, control::Command::Resume).await;
    reach(&actor, Some(false)).await;
    tick(2_000);
    send(&actor, control::Command::StepAway).await;
    reach(&actor, Some(true)).await;
    tick(3_000);
    send(&actor, control::Command::Done).await;
    reach(&actor, None).await;
    let Response::Closed(value) = finish(actor).await else {
        panic!("expected filed evidence");
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let takeovers: Vec<_> = timeline
        .events()
        .iter()
        .filter_map(|event| match event {
            TimelineEvent::Takeover {
                start_ms,
                end_ms,
                paused,
                ..
            } => Some((*start_ms, *end_ms, paused.clone())),
            TimelineEvent::Call { .. }
            | TimelineEvent::CheckMark { .. }
            | TimelineEvent::Reminder { .. }
            | TimelineEvent::HelpRequest { .. } => None,
        })
        .collect();
    assert_eq!(takeovers.len(), 1, "{takeovers:?}");
    let (start, end, paused) = &takeovers[0];
    let at = |offset: u64| Millis(start.0 + offset);
    assert_eq!(*end, at(11_000));
    assert_eq!(
        paused,
        &[
            crate::timeline::PausedSpan {
                start_ms: at(1_000),
                end_ms: at(6_000),
            },
            crate::timeline::PausedSpan {
                start_ms: at(8_000),
                end_ms: at(11_000),
            },
        ]
    );
}
