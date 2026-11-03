use super::*;

#[tokio::test(start_paused = true)]
async fn help_queued_behind_a_held_call_uses_that_takeover() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "queued-during-takeover", false).await;
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
    let (click_reply, click) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "click".into(),
            arguments: json!({"session":"keek-name"}),
            reply: click_reply,
        })
        .await
        .unwrap();
    let (reply, help) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "queued".into(),
            reply,
        })
        .await
        .unwrap();
    actor.clock.elapsed.fetch_add(42000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let click = tokio::time::timeout(Duration::from_secs(1), click)
        .await
        .unwrap()
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(1), help).await;
    let Ok(Ok(Response::ToolResult(help))) = result else {
        panic!("{result:?}")
    };
    let Response::ToolResult(click) = click else {
        panic!("expected hand-back")
    };
    assert_eq!(click["content"][0]["text"], help["content"][0]["text"]);
    assert_eq!(
        help["content"][0]["text"],
        "The developer controlled the session for 0:42. The screen may have changed."
    );
    assert!(matches!(
        events.borrow().control,
        crate::ipc::ControlState::HandBack { .. }
    ));
    assert_eq!(
        actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "omarchy-notification-send")
            .count(),
        0
    );
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn sandbox_end_releases_a_held_call_and_saves_the_takeover() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "ended-takeover", false).await;
    let folder = actor.folder.clone();
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
    let (reply, held) = oneshot::channel();
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
    tokio::task::yield_now().await;
    actor.fail_no_focus.store(true, Ordering::SeqCst);
    actor.ended.send_replace(());
    let result = tokio::time::timeout(Duration::from_secs(1), held).await;
    assert!(
        matches!(result, Ok(Ok(Response::ToolResult(_)))),
        "{result:?}"
    );
    tokio::time::timeout(Duration::from_secs(1), actor.task)
        .await
        .unwrap()
        .unwrap();
    let timeline =
        SessionTimeline::parse(&std::fs::read_to_string(folder.timeline()).unwrap()).unwrap();
    assert_eq!(
        timeline
            .events()
            .iter()
            .filter(|event| matches!(event, TimelineEvent::Takeover { .. }))
            .count(),
        1
    );
    let (reply, response) = oneshot::channel();
    let request = ActorRequest::Close {
        summary: "done".into(),
        reply,
    };
    let session = "ended-takeover".parse().unwrap();
    assert!(!crate::registry::send_actor(
        &actor.sender,
        request,
        &session
    ));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), response).await,
        Ok(Ok(Response::Refused { .. }))
    ));
}
