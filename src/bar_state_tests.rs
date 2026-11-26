use super::*;
use crate::{
    checklist::{IncompleteReason, Mark, SessionEnd, Verdict},
    ipc::{CheckStatus, ControlState, EndedSession, HostShortcuts, SessionState, TakeoverPhase},
    revision::Revision,
    timeline::{CheckId, CheckOutcome, Millis},
    viewer::{CardSpot, SessionInfo, SlotId},
};

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn info(slot: usize) -> SessionInfo {
    SessionInfo {
        name: "billing-12-00-00".parse().unwrap(),
        project: "billing".into(),
        branch: Some("fix/17-invite-role".into()),
        goal: "An invited member gets the Editor role".into(),
        opened: at(0),
        slot: SlotId(slot),
        placement: crate::slots::geometry(
            crate::slots::Slot {
                x: 24,
                y: 50,
                width: 2822,
                height: 1298,
            },
            None,
        ),
        monitor: "DP-1".into(),
        spot: CardSpot::Rail(crate::slots::Slot {
            x: 2856,
            y: 50,
            width: 560,
            height: 384,
        }),
    }
}

fn check(id: u32, outcome: Option<CheckOutcome>) -> CheckStatus {
    CheckStatus {
        id: CheckId(id),
        text: format!("Check {id}"),
        added: false,
        mark: match outcome {
            Some(outcome) => Mark::Outcome {
                outcome,
                comment: String::new(),
                at: Millis(0),
            },
            None => Mark::Unmarked,
        },
    }
}

fn snapshot(control: ControlState, state: SessionState) -> SessionSnapshot {
    SessionSnapshot {
        control,
        state,
        checks: Vec::new(),
        last_action: None,
        reminded: false,
    }
}

fn working() -> SessionSnapshot {
    snapshot(ControlState::Verifier, SessionState::Open)
}

fn ended(end: SessionEnd) -> SessionSnapshot {
    snapshot(
        ControlState::Verifier,
        SessionState::Ended(Box::new(EndedSession {
            end,
            revision: Revision {
                branch: None,
                commit: "e4f09b234".into(),
                dirty: false,
            },
            attempt: None,
            verification: None,
            folder: None,
            video_length: Millis(0),
            actions: 0,
            frame: None,
            elapsed: Millis(60_000),
        })),
    )
}

fn takeover(phase: TakeoverPhase) -> ControlState {
    ControlState::Takeover {
        start_ms: Millis(0),
        shortcuts: HostShortcuts::Live,
        phase,
    }
}

#[test]
fn a_working_session_reads_as_working_with_its_slot_number_and_colour() {
    let session = bar_session(&info(1), &working(), true, at(42));
    assert_eq!(session.n, 2);
    assert_eq!(session.color, "#a77bf3");
    assert_eq!(session.state, BarSessionState::Working);
    assert_eq!(session.project, "billing");
    assert_eq!(session.branch.as_deref(), Some("fix/17-invite-role"));
    assert_eq!(session.opened_at, at(0));
    assert_eq!(session.verdict, None);
    assert!(session.in_focus);
}

#[test]
fn a_help_request_reads_as_needs_you_with_its_reason_as_detail() {
    let control = ControlState::HelpRequest {
        reason: "2FA code".into(),
        start_ms: Millis(0),
    };
    let session = bar_session(
        &info(0),
        &snapshot(control, SessionState::Open),
        false,
        at(42),
    );
    assert_eq!(session.state, BarSessionState::NeedsYou);
    assert_eq!(session.detail, "2FA code");
}

#[test]
fn a_running_takeover_reads_as_takeover() {
    let snapshot = snapshot(takeover(TakeoverPhase::Running), SessionState::Open);
    let session = bar_session(&info(0), &snapshot, false, at(42));
    assert_eq!(session.state, BarSessionState::Takeover);
}

#[test]
fn a_paused_takeover_reads_as_takeover_paused() {
    let paused = TakeoverPhase::Paused {
        since_ms: Millis(0),
    };
    let snapshot = snapshot(takeover(paused), SessionState::Open);
    let session = bar_session(&info(0), &snapshot, false, at(42));
    assert_eq!(session.state, BarSessionState::TakeoverPaused);
}

#[test]
fn an_ended_session_carries_its_verdict() {
    let fixes = ended(SessionEnd::Verdict(Verdict::NeedsFixes));
    let session = bar_session(&info(0), &fixes, false, at(90));
    assert_eq!(session.state, BarSessionState::Ended);
    assert_eq!(session.verdict, Some(BarVerdict::NeedsFixes));

    let incomplete = ended(SessionEnd::Incomplete(IncompleteReason::OpenerLeft));
    let session = bar_session(&info(0), &incomplete, false, at(90));
    assert_eq!(session.verdict, Some(BarVerdict::Incomplete));
}

#[test]
fn checks_encode_one_letter_per_check_in_order() {
    let mut snapshot = working();
    snapshot.checks = vec![
        check(1, Some(CheckOutcome::Passed)),
        check(2, Some(CheckOutcome::Failed)),
        check(3, Some(CheckOutcome::Skipped)),
        check(4, None),
    ];
    let session = bar_session(&info(0), &snapshot, false, at(42));
    assert_eq!(session.checks, "pfs-");
}

#[test]
fn the_file_is_replaced_whole_and_parses_after_two_writes() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("bar.json");
    let state = BarState {
        version: BAR_STATE_VERSION,
        sessions: vec![bar_session(&info(0), &working(), true, at(42))],
    };
    write(&path, &state).unwrap();
    write(&path, &state).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["sessions"][0]["state"], "working");
    assert!(!path.with_extension("tmp").exists());
}

#[test]
fn remove_succeeds_when_the_file_is_absent() {
    let folder = tempfile::tempdir().unwrap();
    remove(&folder.path().join("bar.json")).unwrap();
}
