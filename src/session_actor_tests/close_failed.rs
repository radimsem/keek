use super::*;
#[tokio::test]
async fn close_writes_the_timeline_and_closes_the_driver_when_stop_recording_fails() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", true).await;
    call(&actor, "click").await;
    let closed = actor.closed.clone();
    let calls = actor.calls.clone();
    let events = actor.events.clone();
    let value = assert_incomplete(
        finish(actor).await,
        &events,
        IncompleteReason::RecordingNotStopped,
    );
    assert!(closed.load(Ordering::SeqCst));
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "stop_recording")
    );
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(timeline.events().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_stuck_stop_times_out_and_closes_the_driver() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    actor.stop_hangs.store(true, Ordering::SeqCst);
    let response = close(&actor).await;
    actor.entered.notified().await;
    tokio::time::advance(Duration::from_secs(31)).await;
    let finished = tokio::time::timeout(Duration::from_secs(1), actor.task).await;
    let finished = finished.expect("actor did not finish").unwrap();
    assert!(actor.closed.load(Ordering::SeqCst));
    finished.reply.unwrap().send(finished.response).unwrap();
    assert_incomplete(
        response.await.unwrap(),
        &actor.events,
        IncompleteReason::RecordingNotStopped,
    );
}

#[tokio::test]
async fn a_failed_stop_and_close_file_an_incomplete_session() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", true).await;
    actor.fail_close.store(true, Ordering::SeqCst);
    let events = actor.events.clone();
    assert_incomplete(
        finish(actor).await,
        &events,
        IncompleteReason::RecordingNotStopped,
    );
}

fn assert_incomplete(
    response: Response,
    events: &watch::Receiver<SessionSnapshot>,
    reason: IncompleteReason,
) -> Value {
    let Response::Closed(value) = response else {
        panic!("expected Closed, got {response:?}")
    };
    assert_eq!(value["incomplete"], reason.sentence());
    assert!(value.get("verdict").is_none());
    assert!(!value["checks"].as_array().unwrap().is_empty());
    let report = std::fs::read_to_string(value["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(report.contains(&reason.sentence()), "{report}");
    assert!(!report.contains("## Verdict"), "{report}");
    let SessionState::Ended(card) = &events.borrow().state else {
        panic!("no ended card")
    };
    assert_eq!(card.end, SessionEnd::Incomplete(reason));
    value
}

#[tokio::test]
async fn a_close_error_after_a_good_stop_keeps_the_verdict() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "close-error", false).await;
    actor.fail_close.store(true, Ordering::SeqCst);
    let events = actor.events.clone();
    let Response::Closed(value) = finish(actor).await else {
        panic!("expected Closed")
    };
    assert_eq!(value["verdict"], "Pass");
    let report = std::fs::read_to_string(value["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(report.contains("## Verdict"));
    let SessionState::Ended(card) = &events.borrow().state else {
        panic!("no ended card")
    };
    assert_eq!(
        card.end,
        SessionEnd::Verdict(crate::checklist::Verdict::Pass)
    );
}

#[tokio::test(start_paused = true)]
async fn a_guard_cause_during_stop_overrides_the_timeout() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "guard-timeout", false).await;
    actor.stop_hangs.store(true, Ordering::SeqCst);
    let answer = close(&actor).await;
    actor.entered.notified().await;
    *actor.cause.lock().unwrap() = Some(IncompleteReason::HostWindowOpened);
    tokio::time::advance(Duration::from_secs(30)).await;
    let ended = actor.task.await.unwrap();
    ended.reply.unwrap().send(ended.response).unwrap();
    assert_incomplete(
        answer.await.unwrap(),
        &actor.events,
        IncompleteReason::HostWindowOpened,
    );
}

#[tokio::test(start_paused = true)]
async fn a_guard_cause_with_a_good_stop_overrides_the_verdict() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "guard-stop", false).await;
    actor.stop_hangs.store(true, Ordering::SeqCst);
    let answer = close(&actor).await;
    actor.entered.notified().await;
    *actor.cause.lock().unwrap() = Some(IncompleteReason::HostWindowOpened);
    actor.release.notify_one();
    let ended = actor.task.await.unwrap();
    ended.reply.unwrap().send(ended.response).unwrap();
    assert_incomplete(
        answer.await.unwrap(),
        &actor.events,
        IncompleteReason::HostWindowOpened,
    );
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_call_and_stuck_stop_file_incomplete_at_thirty_seconds() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "abandoned", false).await;
    // Mark before starting the held call so close can decide immediately.
    close_tests::mark(&actor, 1).await.await.unwrap();
    let call = close_tests::slow(&actor).await;
    actor.stop_hangs.store(true, Ordering::SeqCst);
    let mut answer = close_tests::request_close(&actor).await;
    actor.entered.notified().await;
    tokio::time::advance(Duration::from_secs(29)).await;
    assert!(!actor.task.is_finished());
    assert!(matches!(
        answer.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    tokio::time::advance(Duration::from_secs(1)).await;
    let ended = actor.task.await.unwrap();
    ended.reply.unwrap().send(ended.response).unwrap();
    assert_incomplete(
        answer.await.unwrap(),
        &actor.events,
        IncompleteReason::RecordingNotStopped,
    );
    assert!(
        matches!(call.await.unwrap(), Response::Refused { message } if message.contains("closing"))
    );
}

#[tokio::test(start_paused = true)]
async fn a_stop_released_at_twenty_nine_seconds_keeps_the_verdict() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "released", false).await;
    actor.stop_hangs.store(true, Ordering::SeqCst);
    let answer = close(&actor).await;
    actor.entered.notified().await;
    tokio::time::advance(Duration::from_secs(29)).await;
    actor.release.notify_one();
    let events = actor.events.clone();
    let value = close_tests::finished(actor, answer).await;
    assert_eq!(value["verdict"], "Pass");
    let SessionState::Ended(card) = &events.borrow().state else {
        panic!("no ended card")
    };
    assert_eq!(
        card.end,
        SessionEnd::Verdict(crate::checklist::Verdict::Pass)
    );
}
