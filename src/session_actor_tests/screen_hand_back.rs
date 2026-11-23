use super::fake_driver::Screenshot;
use super::*;
use crate::ipc::{ControlState, HandBackResult};
use crate::timeline::HelpOutcome;

const ANSWERED: &str = "If you raised a Help Request, it is answered. The screen may have changed.";
const UNCHANGED: &str = "The screen looks unchanged, so if you raised a Help Request, it may not be answered. Re-read the screen, then call request_help again or skip the Check with a comment.";
const SCALE: &str = "The screenshot is 200 x 100 px of a 1280 x 800 desktop, a scale of 6.4 by 8. Take click coordinates from a fresh get_window_state.";

fn screenshot(block: Option<(i32, i32)>) -> Screenshot {
    let value = super::super::screen::fake_screenshot(200, 100, |pixbuf| {
        pixbuf.fill(0x000000ff);
        if let Some((width, height)) = block {
            pixbuf.new_subpixbuf(0, 0, width, height).fill(0xffffffff);
        }
    });
    Screenshot::Image(
        "image/png",
        Box::leak(
            value["content"][0]["data"]
                .as_str()
                .unwrap()
                .to_owned()
                .into_boxed_str(),
        ),
    )
}

pub(super) fn captures(actor: &RunningActor) -> usize {
    actor
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "get_desktop_state")
        .count()
}

pub(super) async fn wait_for_baseline(actor: &RunningActor, previous: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while captures(actor) == previous {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("expected a Take control screenshot");
}

async fn take_and_finish(
    actor: &RunningActor,
    help: bool,
    before: Screenshot,
    after: Screenshot,
) -> Value {
    let mut events = actor.events.clone();
    let (reply, answer) = oneshot::channel();
    let held_reply = if help {
        actor
            .sender
            .send(ActorRequest::RequestHelp {
                arrival: actor.control.gate.arrival(),
                reason: "2FA code".into(),
                reply,
            })
            .await
            .unwrap();
        while !matches!(
            events.borrow_and_update().control,
            ControlState::HelpRequest { .. }
        ) {
            events.changed().await.unwrap();
        }
        None
    } else {
        Some(reply)
    };
    let previous = captures(actor);
    *actor.screenshot.lock().unwrap() = before;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    if let Some(reply) = held_reply {
        actor
            .sender
            .send(ActorRequest::Call {
                arrival: actor.control.gate.arrival(),
                tool: "click".into(),
                arguments: json!({"session":"keek-name"}),
                reply,
            })
            .await
            .unwrap();
    }
    wait_for_baseline(actor, previous).await;
    *actor.screenshot.lock().unwrap() = after;
    actor.clock.elapsed.fetch_add(42000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let Response::ToolResult(value) = answer.await.unwrap() else {
        panic!("expected a hand-back tool result");
    };
    assert_eq!(captures(actor), previous + 2);
    value
}

fn assert_reply(value: &Value, help: bool, message: &str, after: Screenshot) {
    let reason = if help { " (2FA code)" } else { "" };
    assert_eq!(
        value["content"][0],
        json!({"type":"text","text":format!("The developer pressed Done and handed control back after 0:42{reason}. {message}\n{SCALE}")})
    );
    let Screenshot::Image(mime, data) = after else {
        panic!("expected a PNG fixture");
    };
    assert_eq!(
        value["content"][1],
        json!({"type":"image","mimeType":mime,"data":data})
    );
}

fn assert_card(actor: &RunningActor, expected: HandBackResult) {
    let ControlState::HandBack { result, .. } = actor.events.borrow().control else {
        panic!("expected the HandBack card");
    };
    assert_eq!(result, expected);
}

async fn assert_help_outcomes(actor: RunningActor, expected: &[HelpOutcome]) {
    let Response::Closed(value) = finish(actor).await else {
        panic!("expected filed evidence");
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let outcomes: Vec<_> = timeline
        .events()
        .iter()
        .filter_map(|event| {
            if let TimelineEvent::HelpRequest { outcome, .. } = event {
                Some(*outcome)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(outcomes, expected);
}

#[tokio::test(start_paused = true)]
async fn done_after_a_changed_screen_answers_the_help_request() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "changed-help", false).await;
    let after = screenshot(Some((20, 10)));
    let value = take_and_finish(&actor, true, screenshot(None), after).await;
    assert_reply(&value, true, ANSWERED, after);
    assert_card(&actor, HandBackResult::HelpAnswered);
    assert_help_outcomes(actor, &[HelpOutcome::Answered]).await;
}

#[tokio::test(start_paused = true)]
async fn done_after_an_identical_screen_leaves_the_help_request_unanswered() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "unchanged-help", false).await;
    let image = screenshot(None);
    let value = take_and_finish(&actor, true, image, image).await;
    assert_reply(&value, true, UNCHANGED, image);
    assert_card(&actor, HandBackResult::HelpUnanswered);
    assert_help_outcomes(actor, &[HelpOutcome::Unanswered]).await;
}

#[tokio::test(start_paused = true)]
async fn a_caret_blink_alone_leaves_the_help_request_unanswered() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "blink-help", false).await;
    let after = screenshot(Some((1, 2)));
    let value = take_and_finish(&actor, true, screenshot(None), after).await;
    assert_reply(&value, true, UNCHANGED, after);
    assert_help_outcomes(actor, &[HelpOutcome::Unanswered]).await;
}

#[tokio::test(start_paused = true)]
async fn an_unchanged_screen_releases_a_held_call_without_recording_help() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "unchanged-call", false).await;
    let image = screenshot(None);
    let value = take_and_finish(&actor, false, image, image).await;
    assert_reply(&value, false, UNCHANGED, image);
    assert_card(&actor, HandBackResult::Unchanged);
    assert_help_outcomes(actor, &[]).await;
}

#[tokio::test(start_paused = true)]
async fn a_second_takeover_compares_with_its_own_baseline() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "second-takeover", false).await;
    let image = screenshot(Some((20, 10)));
    let first = take_and_finish(&actor, false, screenshot(None), image).await;
    assert_reply(&first, false, ANSWERED, image);
    assert_card(&actor, HandBackResult::Handed);
    let second = take_and_finish(&actor, false, image, image).await;
    assert_reply(&second, false, UNCHANGED, image);
    assert_card(&actor, HandBackResult::Unchanged);
    assert_help_outcomes(actor, &[]).await;
}

#[tokio::test(start_paused = true)]
async fn an_undecodable_baseline_keeps_the_original_help_reply() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "unreadable-baseline", false).await;
    let after = screenshot(None);
    let value = take_and_finish(&actor, true, Screenshot::Ready, after).await;
    assert_reply(&value, true, ANSWERED, after);
    assert_card(&actor, HandBackResult::HelpAnswered);
    assert_help_outcomes(actor, &[HelpOutcome::Answered]).await;
}
