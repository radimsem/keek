use super::*;
#[tokio::test]
async fn a_driver_ended_by_the_guard_files_the_host_window_reason() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", true).await;
    *actor.cause.lock().unwrap() = Some(IncompleteReason::HostWindowOpened);
    actor.ended.send_replace(());
    let finished = actor.task.await.unwrap();
    let Response::Closed(value) = finished.response else {
        panic!("no Evidence")
    };
    let report = std::fs::read_to_string(value["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(report.contains("keek ended the session because the session opened a window on the developer's desktop other than its Session Window."), "{report}");
    let SessionState::Ended(card) = &actor.events.borrow().state else {
        panic!("no ended card")
    };
    assert_eq!(
        card.end,
        SessionEnd::Incomplete(IncompleteReason::HostWindowOpened)
    );
}
#[tokio::test(start_paused = true)]
async fn an_idle_end_is_not_rewritten_by_the_driver_cause() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", true).await;
    *actor.cause.lock().unwrap() = Some(IncompleteReason::HostWindowOpened);
    tokio::time::advance(Duration::from_secs(300)).await;
    actor.task.await.unwrap();
    let SessionState::Ended(card) = &actor.events.borrow().state else {
        panic!("no ended card")
    };
    assert_eq!(
        card.end,
        SessionEnd::Incomplete(IncompleteReason::Idle {
            limit: Duration::from_secs(300)
        })
    );
}
