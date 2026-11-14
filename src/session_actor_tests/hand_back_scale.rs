use super::fake_driver::Screenshot;
use super::*;

async fn help_reply(actor: &RunningActor) -> Value {
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
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HelpRequest { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let Response::ToolResult(value) = answer.await.unwrap() else {
        panic!("expected hand-back screenshot")
    };
    value
}

#[tokio::test(start_paused = true)]
async fn a_help_reply_states_the_screenshot_size_and_coordinate_factors() {
    for (image, expected) in [
        (
            Screenshot::Image(
                "image/png",
                "iVBORw0KGgoAAAANSUhEUgAAAEAAAAAoCAAAAABrpaM1AAAAGUlEQVR4nO3BgQAAAADDoPlTH+AKVQEAwBsKKAABcWeTzAAAAABJRU5ErkJggg==",
            ),
            "The screenshot is 64 x 40 px of a 1280 x 800 desktop, a scale of 20 by 20. Take click coordinates from a fresh get_window_state.",
        ),
        (
            Screenshot::Image("image/jpeg", "/9j/4AAEYWL/wgALCABQAKABAREA"),
            "The screenshot is 160 x 80 px of a 1280 x 800 desktop, a scale of 8 by 10. Take click coordinates from a fresh get_window_state.",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let actor = start_actor(root.path(), "scaled-help", false).await;
        *actor.screenshot.lock().unwrap() = image;
        let value = help_reply(&actor).await;
        let note = value["content"][0]["text"].as_str().unwrap();
        assert_eq!(note.lines().last(), Some(expected));
        drop(actor.sender);
        actor.task.await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_help_reply_requests_a_fresh_window_state_when_the_screenshot_scale_is_unknown() {
    for image in [
        Screenshot::NoImage,
        Screenshot::Ready,
        Screenshot::Image("image/png", "????"),
        Screenshot::Failed,
    ] {
        let root = tempfile::tempdir().unwrap();
        let actor = start_actor(root.path(), "unknown-scale-help", false).await;
        *actor.screenshot.lock().unwrap() = image;
        let value = help_reply(&actor).await;
        let note = value["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            note.lines().last(),
            Some(
                "The screenshot's scale is unknown. Call get_window_state before acting on coordinates."
            )
        );
        drop(actor.sender);
        actor.task.await.unwrap();
    }
}
