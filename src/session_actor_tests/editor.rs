use super::*;

#[tokio::test]
async fn a_failed_edit_files_raw_recording_without_changing_the_verdict() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    std::fs::write(actor.folder.recording(), b"raw recording").unwrap();
    std::fs::write(
        actor.folder.root().join("verification.mp4"),
        b"planted video",
    )
    .unwrap();
    actor.clock.elapsed.fetch_add(30_000, Ordering::SeqCst);
    let Response::Closed(value) = finish(actor).await else {
        panic!("an edit failure must still file Evidence");
    };
    assert_eq!(value["verdict"], "Pass");
    assert!(
        value["video_error"]
            .as_str()
            .unwrap()
            .contains("fixture ffmpeg failure")
    );
    assert!(
        value["video_error"]
            .as_str()
            .unwrap()
            .contains("Raw Recording is filed")
    );
    assert!(value["evidence"]["verification"].is_null());
    assert_eq!(
        std::fs::read(value["evidence"]["raw"].as_str().unwrap()).unwrap(),
        b"raw recording"
    );
    let report = std::fs::read_to_string(value["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(report.contains("Passed at 0:29"), "{report}");
    assert!(
        report.contains("positions in the Raw Recording"),
        "{report}"
    );
}

#[tokio::test]
async fn a_successful_edit_files_the_video_and_maps_report_times() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    std::fs::write(actor.folder.recording(), b"raw recording").unwrap();
    actor.render_succeeds.store(true, Ordering::SeqCst);
    actor.clock.elapsed.fetch_add(30_000, Ordering::SeqCst);
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused");
    };
    assert_eq!(value["verdict"], "Pass");
    assert!(value.get("video_error").is_none(), "{value}");
    assert_eq!(
        std::fs::read(value["evidence"]["verification"].as_str().unwrap()).unwrap(),
        b"edited recording"
    );
    let report = std::fs::read_to_string(value["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(report.contains("Passed at 0:00"), "{report}");
    assert!(!report.contains("## Verification Video"), "{report}");
}

#[tokio::test]
async fn ending_publishes_filed_evidence_and_the_final_run_duration() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    std::fs::write(actor.folder.recording(), b"raw recording").unwrap();
    actor.render_succeeds.store(true, Ordering::SeqCst);
    actor.clock.elapsed.fetch_add(30_000, Ordering::SeqCst);
    let elapsed = Millis(actor.clock.elapsed.load(Ordering::SeqCst));
    let events = actor.events.clone();
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused")
    };
    let snapshot = events.borrow();
    let SessionState::Ended(end) = &snapshot.state else {
        panic!("not ended: {:?}", snapshot.state)
    };
    assert_eq!(end.elapsed, elapsed);
    assert_eq!(end.end.label(), "Pass");
    assert_eq!(end.attempt, None);
    assert_eq!(
        end.frame.as_deref(),
        Some(root.path().join("frames/app_12-00-00.png").as_path())
    );
    assert_eq!(
        end.verification.as_ref().unwrap().to_str().unwrap(),
        value["evidence"]["verification"].as_str().unwrap()
    );
    assert_eq!(
        end.folder.as_ref().unwrap(),
        end.verification.as_ref().unwrap().parent().unwrap()
    );
    assert!(end.video_length.0 > 0);
}
