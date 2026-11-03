use super::*;

#[tokio::test]
async fn the_timeline_records_each_call_once_and_excludes_setup() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    call(&actor, "read").await;
    call(&actor, "run_steps").await;
    assert_eq!(
        actor.events.borrow().last_action,
        Some(LastAction {
            tool: "run_steps".into(),
            time_ms: Millis(actor.clock.elapsed.load(Ordering::SeqCst) - ACTION_DURATION_MS)
        })
    );
    let folder = actor.folder.clone();
    let calls = actor.calls.clone();
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused");
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        timeline.events(),
        [
            TimelineEvent::Call {
                tool: "read".into(),
                action: false,
                start_ms: Millis(0),
                end_ms: Millis(0),
                is_error: false
            },
            TimelineEvent::Call {
                tool: "run_steps".into(),
                action: true,
                start_ms: Millis(0),
                end_ms: Millis(184),
                is_error: false
            },
            TimelineEvent::CheckMark {
                check: crate::timeline::CheckId(1),
                outcome: crate::timeline::CheckOutcome::Passed,
                at_ms: Millis(184),
            },
        ]
    );
    let calls = calls.lock().unwrap();
    let recording = calls
        .iter()
        .find(|(tool, _)| tool == "start_recording")
        .unwrap();
    assert_eq!(
        recording.1,
        json!({"output_dir":folder.root(),"record_video":true})
    );
    assert!(
        calls
            .iter()
            .filter(|(tool, _)| tool == "read" || tool == "run_steps")
            .all(|(_, args)| args.get("session").is_none())
    );
}
