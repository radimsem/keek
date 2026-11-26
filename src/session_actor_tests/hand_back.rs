use super::takeover::forward;
use super::*;

#[tokio::test(start_paused = true)]
async fn a_forwarded_call_during_takeover_returns_a_note_and_fresh_screenshot_without_acting() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "takeover", false).await;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let held = tokio::spawn({
        let sender = actor.sender.clone();
        let gate = actor.control.gate.clone();
        async move { forward(&sender, &gate, "click").await }
    });
    tokio::task::yield_now().await;
    assert!(!held.is_finished());
    actor.clock.elapsed.fetch_add(42000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let Response::ToolResult(value) = held.await.unwrap() else {
        panic!("expected tool result")
    };
    assert_eq!(
        value["content"][0]["text"],
        "The developer pressed Done and handed control back after 0:42. If you raised a Help Request, it is answered. The screen may have changed.\nThe screenshot's scale is unknown. Call get_window_state before acting on coordinates."
    );
    assert!(
        actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool != "click")
    );
    assert!(
        actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "get_desktop_state")
    );
    drop(actor.sender);
    actor.task.await.unwrap();
    let timeline: SessionTimeline =
        serde_json::from_slice(&std::fs::read(actor.folder.root().join("timeline.json")).unwrap())
            .unwrap();
    assert_eq!(
        timeline
            .events()
            .iter()
            .filter(|event| matches!(event, TimelineEvent::Takeover { .. }))
            .count(),
        1
    );
    assert!(
        timeline.events().iter().all(
            |event| !matches!(event,TimelineEvent::Call{tool,..} if tool=="get_desktop_state")
        )
    );
}
#[tokio::test(start_paused = true)]
async fn a_help_request_notifies_once_until_take_control_and_waits_for_done_without_a_timeout() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "help", false).await;
    let (reply, mut answer) = oneshot::channel();
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
    tokio::time::advance(Duration::from_secs(7200)).await;
    assert!(answer.try_recv().is_err());
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
    actor.clock.elapsed.fetch_add(42000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let Response::ToolResult(value) = answer.await.unwrap() else {
        panic!("expected tool result")
    };
    assert_eq!(
        value["content"][0]["text"],
        "The developer pressed Done and handed control back after 0:42 (2FA code). If you raised a Help Request, it is answered. The screen may have changed.\nThe screenshot's scale is unknown. Call get_window_state before acting on coordinates."
    );
    let calls = actor.calls.lock().unwrap().clone();
    let notifications: Vec<_> = calls
        .iter()
        .filter(|(tool, _)| tool == "omarchy-notification-send")
        .collect();
    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].1.to_string().contains("2FA code · app"));
    assert!(
        notifications[0]
            .1
            .to_string()
            .contains(r#""-u","critical","-i","keek","Agent needs you""#)
    );
    assert!(!notifications[0].1.to_string().contains("--app-name"));
    let dismissed: Vec<_> = calls
        .iter()
        .filter(|(tool, _)| tool == "omarchy-notification-dismiss")
        .collect();
    assert_eq!(dismissed.len(), 1);
    assert_eq!(dismissed[0].1, json!(["Agent needs you"]));
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn fresh_hand_back_content_is_masked_and_follows_the_note() {
    let root = tempfile::tempdir().unwrap();
    let credentials =
        crate::credentials::Credentials::parse(Path::new("fixture.env"), "PASSWORD=made-up-value")
            .unwrap();
    let (actor, _) =
        start_actor_with_credentials(root.path(), "masked-help", false, None, credentials).await;
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
        panic!("expected screenshot")
    };
    assert!(
        value["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("The developer")
    );
    assert_eq!(value["content"][1]["text"], "[masked]");
    assert_eq!(value["content"][2]["type"], "image");
    drop(actor.sender);
    actor.task.await.unwrap();
}
