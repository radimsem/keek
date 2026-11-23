use super::*;
use crate::{
    checklist::{SessionEnd, Verdict},
    ipc::{CheckStatus, ControlState, HostShortcuts},
    timeline::{CheckId, Millis},
};
fn open_with(marks: &[Option<CheckOutcome>]) -> SessionSnapshot {
    SessionSnapshot {
        control: ControlState::Verifier,
        state: SessionState::Open,
        checks: marks
            .iter()
            .enumerate()
            .map(|(n, outcome)| CheckStatus {
                added: false,
                id: CheckId(n as u32 + 1),
                text: "check".into(),
                mark: match outcome {
                    Some(outcome) => Mark::Outcome {
                        outcome: *outcome,
                        comment: String::new(),
                        at: Millis(0),
                    },
                    None => Mark::Unmarked,
                },
            })
            .collect(),
        last_action: None,
        reminded: false,
    }
}
fn ticks(s: &SessionSnapshot) -> Vec<Tick> {
    tests::render(s, 0, false).checked.unwrap().ticks
}
const PASSED: Option<CheckOutcome> = Some(CheckOutcome::Passed);
#[test]
fn the_first_unmarked_check_is_ringed_while_the_verifier_works() {
    let s = open_with(&[PASSED, None, None]);
    assert_eq!(ticks(&s), [Tick::Passed, Tick::InProgress, Tick::Unmarked]);
    assert_eq!(
        tests::render(&s, 0, false).checked.unwrap().accessible,
        "1 passed, 0 failed, 0 skipped, 2 not checked yet"
    );
}
#[test]
fn the_ring_moves_to_the_next_unmarked_check_as_marks_arrive() {
    let s = open_with(&[None, PASSED, None]);
    assert_eq!(ticks(&s), [Tick::InProgress, Tick::Passed, Tick::Unmarked]);
    let s = open_with(&[PASSED, PASSED, None]);
    assert_eq!(ticks(&s), [Tick::Passed, Tick::Passed, Tick::InProgress]);
}
#[test]
fn no_check_is_ringed_once_every_check_is_marked() {
    let s = open_with(&[PASSED, Some(CheckOutcome::Skipped)]);
    assert_eq!(ticks(&s), [Tick::Passed, Tick::Skipped]);
}
#[test]
fn a_check_beyond_the_eight_ticks_is_not_drawn_with_a_ring() {
    let mut marks = vec![PASSED; 8];
    marks.push(None);
    assert!(!ticks(&open_with(&marks)).contains(&Tick::InProgress));
}
#[test]
fn the_ring_stays_while_the_verifier_is_handed_back_control() {
    let mut s = open_with(&[None]);
    s.control = ControlState::HandBack {
        end_ms: Millis(0),
        result: crate::ipc::HandBackResult::Handed,
        length: Millis(1000),
    };
    assert_eq!(ticks(&s), [Tick::InProgress]);
}
#[test]
fn no_check_is_ringed_during_a_help_request_a_takeover_or_outside_an_open_session() {
    let mut help = open_with(&[None]);
    help.control = ControlState::HelpRequest {
        reason: "2FA code".into(),
        start_ms: Millis(0),
    };
    let mut takeover = open_with(&[None]);
    takeover.control = ControlState::Takeover {
        start_ms: Millis(0),
        shortcuts: HostShortcuts::Captured,
        phase: crate::ipc::TakeoverPhase::Running,
    };
    let mut opening = open_with(&[None]);
    opening.state = SessionState::Opening;
    let mut closing = open_with(&[None]);
    closing.state = SessionState::Closing;
    let mut ended = super::super::super::verdict_view::tests::ended(SessionEnd::Verdict(
        Verdict::PartlyVerified,
    ));
    ended.checks = open_with(&[None]).checks;
    for s in [help, takeover, opening, closing, ended] {
        assert_eq!(ticks(&s), [Tick::Unmarked]);
    }
}
#[test]
fn a_reminder_shows_its_state_over_the_actions_since_the_last_mark() {
    let mut s = open_with(&[None]);
    s.reminded = true;
    let view = tests::render(&s, 0, false);
    assert_eq!(view.label, StateLabel::Reminded);
    assert_eq!(view.label.text(), "Reminded the Verifier to mark Checks");
    assert_eq!(view.label.css_class(), None);
    assert_eq!(view.detail, "8 actions since the last mark");
    s.reminded = false;
    assert_eq!(tests::render(&s, 0, false).label, StateLabel::LastAction);
}
#[test]
fn a_help_request_a_takeover_and_the_view_only_hint_outrank_the_reminder() {
    let mut s = open_with(&[None]);
    s.reminded = true;
    assert_eq!(tests::render(&s, 0, true).label, StateLabel::ViewOnly);
    s.control = ControlState::HelpRequest {
        reason: "2FA code".into(),
        start_ms: Millis(0),
    };
    assert_eq!(tests::render(&s, 0, false).label, StateLabel::NeedsYou);
    s.control = ControlState::Takeover {
        start_ms: Millis(0),
        shortcuts: HostShortcuts::Captured,
        phase: crate::ipc::TakeoverPhase::Running,
    };
    assert_eq!(tests::render(&s, 0, false).label, StateLabel::Takeover);
}
