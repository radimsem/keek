use super::*;

#[tokio::test]
async fn a_stopped_desktop_files_incomplete_evidence() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", true).await;
    actor.fail_close.store(true, Ordering::SeqCst);
    actor.ended.send(()).unwrap();
    let finished = actor.task.await.unwrap();
    let Response::Closed(value) = finished.response else {
        panic!("no Evidence: {:?}", finished.response)
    };
    assert!(
        value["evidence"]["report"]
            .as_str()
            .unwrap()
            .contains("incomplete")
    );
}

#[tokio::test(start_paused = true)]
async fn the_idle_timer_ends_the_session_at_the_limit() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    tokio::time::advance(Duration::from_secs(299)).await;
    tokio::task::yield_now().await;
    assert_eq!(actor.events.borrow().state, SessionState::Open);
    assert!(!actor.task.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    let finished = actor.task.await.unwrap();
    assert!(matches!(finished.response, Response::Closed(_)));
}

#[tokio::test(start_paused = true)]
async fn a_tool_call_restarts_the_full_idle_limit() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    tokio::time::advance(Duration::from_secs(200)).await;
    call(&actor, "click").await;
    tokio::time::advance(Duration::from_secs(100)).await;
    tokio::task::yield_now().await;
    assert_eq!(actor.events.borrow().state, SessionState::Open);
    assert!(!actor.task.is_finished());
    tokio::time::advance(Duration::from_secs(200)).await;
    assert!(matches!(
        actor.task.await.unwrap().response,
        Response::Closed(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn takeover_pauses_idle_until_control_returns() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
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
    tokio::time::advance(Duration::from_secs(1000)).await;
    tokio::task::yield_now().await;
    assert_eq!(actor.events.borrow().state, SessionState::Open);
    assert!(!actor.task.is_finished());
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HandBack { .. }
    ) {
        events.changed().await.unwrap();
    }
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(299)).await;
    tokio::task::yield_now().await;
    assert_eq!(actor.events.borrow().state, SessionState::Open);
    assert!(!actor.task.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        actor.task.await.unwrap().response,
        Response::Closed(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_help_request_pauses_idle_until_help_finishes() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "Sign in".into(),
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
    tokio::time::advance(Duration::from_secs(1000)).await;
    tokio::task::yield_now().await;
    assert_eq!(actor.events.borrow().state, SessionState::Open);
    assert!(!actor.task.is_finished());
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
    answer.await.unwrap();
    tokio::time::advance(Duration::from_secs(299)).await;
    tokio::task::yield_now().await;
    assert_eq!(actor.events.borrow().state, SessionState::Open);
    assert!(!actor.task.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        actor.task.await.unwrap().response,
        Response::Closed(_)
    ));
}

#[tokio::test]
async fn disconnect_interrupts_the_handback_screenshot() {
    let root = tempfile::tempdir().unwrap();
    let (alive, opener) = watch::channel(());
    let (actor, _) = start_configured(
        root.path(),
        "app_12-00-00",
        false,
        None,
        Default::default(),
        Duration::from_secs(300),
        Some(opener),
    )
    .await;
    let previous = super::screen_hand_back::captures(&actor);
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
    super::screen_hand_back::wait_for_baseline(&actor, previous).await;
    *actor.screenshot.lock().unwrap() = fake_driver::Screenshot::Held;
    let (reply, _answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "click".into(),
            arguments: json!({}),
            reply,
        })
        .await
        .unwrap();
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HandBack { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor.entered.notified().await;
    drop(alive);
    let finished = tokio::time::timeout(Duration::from_secs(1), actor.task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(finished.response, Response::Closed(_)));
}

#[tokio::test(start_paused = true)]
async fn idle_does_not_interrupt_the_handback_screenshot() {
    let root = tempfile::tempdir().unwrap();
    let (actor, _) = start_configured(
        root.path(),
        "app_12-00-00",
        false,
        None,
        Default::default(),
        Duration::from_secs(3),
        None,
    )
    .await;
    let previous = super::screen_hand_back::captures(&actor);
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
    super::screen_hand_back::wait_for_baseline(&actor, previous).await;
    *actor.screenshot.lock().unwrap() = fake_driver::Screenshot::Held;
    let (reply, mut answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "click".into(),
            arguments: json!({}),
            reply,
        })
        .await
        .unwrap();
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    actor.entered.notified().await;
    tokio::time::advance(Duration::from_secs(4)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(answer.try_recv().is_err());
    actor.release.notify_one();
    assert!(matches!(answer.await.unwrap(), Response::ToolResult(_)));
    actor.task.abort();
    let _ = actor.task.await;
}
