use std::path::{Path, PathBuf};

use anyhow::Result;
use keek::session_folder::{SessionFolder, SessionName, Turn};
use keek::timeline::{Millis, SessionTimeline, TimelineError};

const FIXTURE_SESSION: &str = "myapp_14-03-22";

fn fixture_sessions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions")
}

fn fixture_folder() -> Result<SessionFolder> {
    Ok(SessionFolder::new(
        fixture_sessions_dir(),
        &FIXTURE_SESSION.parse::<SessionName>()?,
    ))
}

#[test]
fn timeline_fixture_round_trips_without_changing_any_field() -> Result<()> {
    let text = std::fs::read_to_string(fixture_folder()?.timeline())?;
    let timeline = SessionTimeline::parse(&text)?;

    let written = serde_json::to_value(&timeline)?;
    let original: serde_json::Value = serde_json::from_str(&text)?;

    assert_eq!(written, original);
    assert_eq!(timeline.video_lead(), Millis(50));
    Ok(())
}

#[test]
fn timeline_refuses_an_unknown_schema_version() {
    let result =
        SessionTimeline::parse(r#"{"schema_version": 2, "video_lead_ms": 50, "events": []}"#);

    assert!(matches!(result, Err(TimelineError::UnsupportedSchema(2))));
}

#[test]
fn session_folder_names_every_path_in_the_fixture() -> Result<()> {
    let folder = fixture_folder()?;

    for path in [
        folder.recording(),
        folder.timeline(),
        folder.turn(Turn(1)),
        folder.turn(Turn(2)),
        folder.action(Turn(1)),
        folder.action(Turn(2)),
    ] {
        assert!(path.exists(), "missing fixture path {}", path.display());
    }
    assert_eq!(
        folder
            .turn(Turn(12))
            .file_name()
            .and_then(|name| name.to_str()),
        Some("turn-00012"),
    );
    Ok(())
}

#[test]
fn session_folder_round_trips_through_serde() -> Result<()> {
    let folder = fixture_folder()?;

    let json = serde_json::to_string(&folder)?;

    assert_eq!(serde_json::from_str::<SessionFolder>(&json)?, folder);
    Ok(())
}

#[test]
fn session_name_round_trips_through_serde_and_stays_validated() -> Result<()> {
    let name: SessionName = FIXTURE_SESSION.parse()?;

    let json = serde_json::to_string(&name)?;

    assert_eq!(json, format!("\"{FIXTURE_SESSION}\""));
    assert_eq!(serde_json::from_str::<SessionName>(&json)?, name);
    assert!(serde_json::from_str::<SessionName>("\"../escape\"").is_err());
    Ok(())
}

#[test]
fn session_name_refuses_anything_that_could_leave_the_sessions_folder() {
    for name in ["", ".", "..", "a/b", "../escape", "nul\0byte"] {
        assert!(name.parse::<SessionName>().is_err(), "accepted {name:?}");
    }
    assert!(FIXTURE_SESSION.parse::<SessionName>().is_ok());
}
