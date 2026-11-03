use super::*;

pub(super) async fn mark(actor: &RunningActor, check: u32) -> oneshot::Receiver<Response> {
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::MarkCheck {
            check: CheckId(check),
            outcome: CheckOutcome::Passed,
            comment: String::new(),
            reply,
        })
        .await
        .unwrap();
    answer
}
pub(super) async fn request_close(actor: &RunningActor) -> oneshot::Receiver<Response> {
    let (reply, answer) = oneshot::channel();
    actor.closing.store(true, Ordering::SeqCst);
    actor
        .close
        .send(CloseRequest {
            summary: "done".into(),
            reply,
        })
        .await
        .unwrap();
    answer
}
pub(super) async fn slow(actor: &RunningActor) -> oneshot::Receiver<Response> {
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "slow".into(),
            arguments: json!({}),
            reply,
        })
        .await
        .unwrap();
    actor.entered.notified().await;
    answer
}
pub(super) async fn finished(actor: RunningActor, answer: oneshot::Receiver<Response>) -> Value {
    let ended = actor.task.await.unwrap();
    ended.reply.unwrap().send(ended.response).unwrap();
    let Response::Closed(value) = answer.await.unwrap() else {
        panic!("expected Verdict")
    };
    value
}

#[tokio::test(start_paused = true)]
async fn close_stops_a_call_in_flight_and_files_the_verdict() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "close-fast", false).await;
    mark(&actor, 1).await.await.unwrap();
    let _call = slow(&actor).await;
    let calls = actor.calls.clone();
    let closed = actor.closed.clone();
    let answer = request_close(&actor).await;
    let start = tokio::time::Instant::now();
    finished(actor, answer).await;
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "close waited for the call"
    );
    assert!(closed.load(Ordering::SeqCst));
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "driver close")
            .count(),
        1
    );
    let calls = calls.lock().unwrap();
    assert!(!calls.iter().any(|(tool, _)| tool == "driver finished"));
    assert_eq!(
        calls
            .iter()
            .filter(|(tool, _)| tool == "stop_recording")
            .count(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn the_abandoned_call_is_refused_and_recorded_as_an_error() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "close-error", false).await;
    mark(&actor, 1).await.await.unwrap();
    let call = slow(&actor).await;
    let answer = request_close(&actor).await;
    let value = finished(actor, answer).await;
    let Response::Refused { message } = call.await.unwrap() else {
        panic!("expected refusal")
    };
    assert!(
        message.contains("closing") && message.contains("slow"),
        "{message}"
    );
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert!(timeline.events().iter().any(
        |event| matches!(event, TimelineEvent::Call { tool, is_error: true, .. } if tool == "slow")
    ));
}

#[tokio::test(start_paused = true)]
async fn close_refuses_the_calls_queued_behind_the_one_in_flight() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "close-queue", false).await;
    mark(&actor, 1).await.await.unwrap();
    let _call = slow(&actor).await;
    let mut queued = vec![];
    for _ in 0..32 {
        let (reply, answer) = oneshot::channel();
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
        queued.push(answer);
    }
    // The separate close path must be accepted even when every mailbox slot is occupied.
    let answer = tokio::time::timeout(Duration::from_secs(1), request_close(&actor))
        .await
        .expect("close was blocked by a full mailbox");
    let calls = actor.calls.clone();
    finished(actor, answer).await;
    for answer in queued {
        assert!(
            matches!(answer.await.unwrap(), Response::Refused { message } if message.contains("is closing"))
        );
    }
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "click")
    );
}

#[tokio::test(start_paused = true)]
async fn a_refused_close_leaves_the_call_running_and_a_retry_succeeds() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "close-retry", false).await;
    let mut call = slow(&actor).await;
    let answer = request_close(&actor).await;
    let result = tokio::time::timeout(Duration::from_secs(1), answer).await;
    assert!(
        matches!(result, Ok(Ok(Response::Refused { message })) if message.contains("fixture works"))
    );
    assert!(call.try_recv().is_err());
    actor.release.notify_one();
    assert!(matches!(call.await.unwrap(), Response::ToolResult(_)));
    let answer = close(&actor).await;
    finished(actor, answer).await;
}

#[tokio::test(start_paused = true)]
async fn queued_marks_are_applied_before_the_close_decides() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "close-marks", false).await;
    let (reply, added) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::AddCheck {
            text: "second".into(),
            reply,
        })
        .await
        .unwrap();
    added.await.unwrap();
    let _call = slow(&actor).await;
    let first = mark(&actor, 1).await;
    let second = mark(&actor, 2).await;
    let answer = request_close(&actor).await;
    let start = tokio::time::Instant::now();
    finished(actor, answer).await;
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(matches!(first.await.unwrap(), Response::CheckMarked { .. }));
    assert!(matches!(
        second.await.unwrap(),
        Response::CheckMarked { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn an_idle_actor_takes_close_before_queued_calls() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "close-idle", false).await;
    mark(&actor, 1).await.await.unwrap();
    let (reply, call) = oneshot::channel();
    actor
        .sender
        .try_send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "click".into(),
            arguments: json!({}),
            reply,
        })
        .unwrap();
    let answer = request_close(&actor).await;
    let calls = actor.calls.clone();
    finished(actor, answer).await;
    assert!(matches!(call.await.unwrap(), Response::Refused { .. }));
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "click")
    );
}

#[tokio::test(start_paused = true)]
async fn a_close_waits_for_a_held_help_request() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "close-help", false).await;
    mark(&actor, 1).await.await.unwrap();
    let (reply, help) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "help".into(),
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
    let mut answer = request_close(&actor).await;
    tokio::time::advance(Duration::from_secs(500)).await;
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
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    assert!(matches!(help.await.unwrap(), Response::ToolResult(_)));
    finished(actor, answer).await;
}

#[tokio::test(start_paused = true)]
async fn a_pending_close_finishes_when_the_opener_sender_drops() {
    let root = tempfile::tempdir().unwrap();
    let (alive, opener) = watch::channel(());
    let (actor, _) = start_configured(
        root.path(),
        "close-opener",
        false,
        None,
        Default::default(),
        Duration::from_secs(300),
        Some(opener),
    )
    .await;
    mark(&actor, 1).await.await.unwrap();
    let call = slow(&actor).await;
    // The controller may already be waiting for the in-flight call's permit.
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    tokio::task::yield_now().await;
    let answer = request_close(&actor).await;
    drop(alive);
    let start = tokio::time::Instant::now();
    finished(actor, answer).await;
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(
        matches!(call.await.unwrap(), Response::Refused { message } if message.contains("closing"))
    );
}

#[tokio::test(start_paused = true)]
async fn a_refused_close_restarts_the_idle_timer() {
    let root = tempfile::tempdir().unwrap();
    let (actor, _) = start_configured(
        root.path(),
        "close-idle-retry",
        false,
        None,
        Default::default(),
        Duration::from_secs(300),
        None,
    )
    .await;
    tokio::time::advance(Duration::from_secs(299)).await;
    let answer = request_close(&actor).await;
    assert!(
        matches!(answer.await.unwrap(), Response::Refused { message } if message.contains("fixture works"))
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    let (reply, marked) = oneshot::channel();
    actor
        .sender
        .try_send(ActorRequest::MarkCheck {
            check: CheckId(1),
            outcome: CheckOutcome::Passed,
            comment: String::new(),
            reply,
        })
        .unwrap();
    assert!(
        matches!(marked.await.unwrap(), Response::CheckMarked { .. }),
        "the refused close left the old idle deadline active"
    );
    let answer = request_close(&actor).await;
    finished(actor, answer).await;
}

#[tokio::test(start_paused = true)]
async fn a_refused_close_during_a_call_keeps_the_idle_timer_restarted() {
    let root = tempfile::tempdir().unwrap();
    let (actor, _) = start_configured(
        root.path(),
        "close-inflight-idle",
        false,
        None,
        Default::default(),
        Duration::from_secs(300),
        None,
    )
    .await;
    tokio::time::advance(Duration::from_secs(299)).await;
    let call = slow(&actor).await;
    let answer = request_close(&actor).await;
    assert!(
        matches!(answer.await.unwrap(), Response::Refused { message } if message.contains("fixture works"))
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    actor.release.notify_one();
    assert!(matches!(call.await.unwrap(), Response::ToolResult(_)));
    tokio::time::advance(Duration::from_secs(299)).await;
    assert!(matches!(
        mark(&actor, 1).await.await.unwrap(),
        Response::CheckMarked { .. }
    ));
    let answer = request_close(&actor).await;
    finished(actor, answer).await;
}
