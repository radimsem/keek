use super::*;

async fn wait_for(
    events: &mut watch::Receiver<SessionSnapshot>,
    wanted: fn(&crate::ipc::ControlState) -> bool,
) {
    let waiting = async {
        while !wanted(&events.borrow_and_update().control) {
            events.changed().await.unwrap();
        }
    };
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the card never reached the expected state");
}

async fn raise_help(actor: &RunningActor) -> oneshot::Receiver<Response> {
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    answer
}

fn border_values(actor: &RunningActor) -> Vec<String> {
    actor
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(program, _)| program == "hyprctl")
        .map(|(_, args)| args.to_string())
        .filter(|args| args.contains("inactive_border_color"))
        .collect()
}

fn notifications(actor: &RunningActor) -> usize {
    actor
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(program, _)| program == "omarchy-notification-send")
        .count()
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_help_request_returns_the_card_to_the_verifier_and_clears_the_border() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "withdraw", false).await;
    let mut events = actor.events.clone();
    let answer = raise_help(&actor).await;
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::HelpRequest { .. })
    })
    .await;
    settle().await;
    assert!(border_values(&actor).last().unwrap().contains("rgb("));

    drop(answer);
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::Verifier)
    })
    .await;
    settle().await;

    assert!(border_values(&actor).last().unwrap().contains("unset"));
    assert!(!actor.task.is_finished());
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_help_request_returns_the_card_to_the_hand_back_it_showed() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "withdraw-hand-back", false).await;
    let mut events = actor.events.clone();
    for command in [control::Command::TakeControl, control::Command::Done] {
        actor.control.commands.send(command).await.unwrap();
    }
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::HandBack { .. })
    })
    .await;
    let handed_back = events.borrow().control.clone();
    let answer = raise_help(&actor).await;
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::HelpRequest { .. })
    })
    .await;

    drop(answer);
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::HandBack { .. })
    })
    .await;

    assert_eq!(events.borrow().control, handed_back);
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_call_after_a_long_withdrawn_help_request_runs_without_a_takeover() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "withdraw-then-call", false).await;
    let mut events = actor.events.clone();
    let answer = raise_help(&actor).await;
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::HelpRequest { .. })
    })
    .await;
    tokio::time::advance(Duration::from_secs(600)).await;
    drop(answer);
    settle().await;

    let response = tokio::time::timeout(Duration::from_secs(5), call(&actor, "click"))
        .await
        .expect("the call waited for a Takeover");

    let Response::ToolResult(value) = response else {
        panic!("expected a tool result, got {response:?}")
    };
    assert!(
        !value["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("developer controlled")
    );
    assert!(matches!(
        events.borrow().control,
        crate::ipc::ControlState::Verifier
    ));
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_help_request_cancelled_while_queued_is_never_raised() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "withdraw-queued", false).await;
    let (slow_reply, slow) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "slow".into(),
            arguments: json!({"session":"keek-name"}),
            reply: slow_reply,
        })
        .await
        .unwrap();
    actor.entered.notified().await;
    let answer = raise_help(&actor).await;
    drop(answer);
    actor.release.notify_one();
    slow.await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), call(&actor, "click"))
        .await
        .expect("the call waited behind a Help Request");

    assert_eq!(notifications(&actor), 0);
    assert!(border_values(&actor).is_empty());
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_help_request_cancelled_after_take_control_leaves_the_takeover_running() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "withdraw-takeover", false).await;
    let mut events = actor.events.clone();
    let answer = raise_help(&actor).await;
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::HelpRequest { .. })
    })
    .await;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::Takeover { .. })
    })
    .await;

    drop(answer);
    settle().await;
    assert!(matches!(
        events.borrow().control,
        crate::ipc::ControlState::Takeover { .. }
    ));

    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    wait_for(&mut events, |c| {
        matches!(c, crate::ipc::ControlState::HandBack { .. })
    })
    .await;
    let response = tokio::time::timeout(Duration::from_secs(5), call(&actor, "click"))
        .await
        .expect("the session stopped serving calls");
    assert!(matches!(response, Response::ToolResult(_)));
    drop(actor.sender);
    actor.task.await.unwrap();
}
