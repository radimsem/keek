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
