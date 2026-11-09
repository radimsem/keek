use super::*;

fn keepalives(actor: &RunningActor) -> usize {
    actor
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "get_recording_state")
        .count()
}

async fn wait_minutes(minutes: u32) {
    for _ in 0..minutes {
        tokio::time::advance(Duration::from_secs(60)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }
}

async fn reach(actor: &RunningActor, takeover: bool) {
    let mut events = actor.events.clone();
    loop {
        let reached = match events.borrow_and_update().control {
            crate::ipc::ControlState::Takeover { .. } => takeover,
            crate::ipc::ControlState::HelpRequest { .. } => !takeover,
            _ => false,
        };
        if reached {
            return;
        }
        events.changed().await.unwrap();
    }
}

async fn take_control(actor: &RunningActor) {
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    reach(actor, true).await;
}

#[tokio::test(start_paused = true)]
async fn an_open_help_request_keeps_the_recording_alive_until_done() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "help-keepalive", false).await;
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
    reach(&actor, false).await;
    wait_minutes(5).await;
    let waiting = keepalives(&actor);
    assert!(
        waiting >= 4,
        "{waiting} keepalives in 5 minutes of a Help Request"
    );
    take_control(&actor).await;
    wait_minutes(5).await;
    assert!(keepalives(&actor) >= waiting + 4);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    answer.await.unwrap();
    let done = keepalives(&actor);
    wait_minutes(3).await;
    assert_eq!(keepalives(&actor), done);
}

#[tokio::test(start_paused = true)]
async fn a_takeover_keeps_the_recording_alive_with_or_without_a_held_call() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "takeover-keepalive", false).await;
    take_control(&actor).await;
    wait_minutes(5).await;
    let idle = keepalives(&actor);
    assert!(idle >= 4, "{idle} keepalives in 5 minutes of a Takeover");
    let held = tokio::spawn({
        let sender = actor.sender.clone();
        let gate = actor.control.gate.clone();
        async move { super::takeover::forward(&sender, &gate, "click").await }
    });
    wait_minutes(5).await;
    assert!(!held.is_finished());
    assert!(keepalives(&actor) >= idle + 4);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    held.await.unwrap();
}
