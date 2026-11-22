use super::*;
use crate::{
    checklist::{IncompleteReason, Mark, SessionEnd, Verdict},
    ipc::{CheckStatus, EndedSession, Percent, SessionSnapshot, SessionState},
    revision::Revision,
    timeline::{CheckId, CheckOutcome, Millis},
};
pub(crate) fn ended(end: SessionEnd) -> SessionSnapshot {
    SessionSnapshot {
        control: Default::default(),
        state: SessionState::Ended(Box::new(EndedSession {
            end,
            revision: Revision {
                branch: Some("fix/17-login-redirect".into()),
                commit: "e4f09b234".into(),
                dirty: false,
            },
            attempt: Some(crate::attempts::Attempt(1)),
            verification: Some("/video.mp4".into()),
            folder: Some("/evidence".into()),
            video_length: Millis(38000),
            actions: 14,
            frame: None,
            elapsed: Millis(182000),
        })),
        checks: vec![CheckStatus {
            id: CheckId(1),
            text: "Signing in lands on /dashboard".into(),
            added: true,
            mark: Mark::Outcome {
                outcome: CheckOutcome::Passed,
                comment: String::new(),
                at: Millis(100),
            },
        }],
        reminded: false,
        last_action: None,
    }
}
#[test]
fn rendering_shows_the_whole_percentage() {
    let s = SessionSnapshot {
        state: SessionState::Rendering {
            percent: Percent(64),
            frame: None,
            elapsed: Millis(0),
        },
        reminded: false,
        ..ended(SessionEnd::Verdict(Verdict::Pass))
    };
    let v = for_snapshot(&s).unwrap();
    assert_eq!(v.title, "Rendering the verification video");
    assert_eq!(v.tally, "64%");
    assert!(v.rows.is_empty());
}
#[test]
fn pass_shows_revision_passed_tally_and_added_checks() {
    let v = for_snapshot(&ended(SessionEnd::Verdict(Verdict::Pass))).unwrap();
    assert_eq!(v.title, "Pass");
    assert_eq!(v.subtitle, "Verified fix/17-login-redirect at e4f09b2");
    assert_eq!(v.tally, "1 passed");
    assert!(v.rows[0].added);
    assert_eq!(v.rows[0].tick, super::super::footer_view::Tick::Passed);
    assert_eq!(v.rows[0].comment, None);
}
#[test]
fn needs_fixes_shows_failed_and_skipped_comments_and_attempt() {
    let mut s = ended(SessionEnd::Verdict(Verdict::NeedsFixes));
    s.checks[0].mark = Mark::Outcome {
        outcome: CheckOutcome::Failed,
        comment: "Header only".into(),
        at: Millis(0),
    };
    s.checks.push(CheckStatus {
        id: CheckId(2),
        text: "Currency".into(),
        added: false,
        mark: Mark::Outcome {
            outcome: CheckOutcome::Skipped,
            comment: "No EUR data".into(),
            at: Millis(0),
        },
    });
    let v = for_snapshot(&s).unwrap();
    assert_eq!(v.title, "Needs fixes");
    assert_eq!(v.subtitle, "Checked fix/17-login-redirect at e4f09b2");
    assert_eq!(v.tally, "1 failed, 1 skipped · attempt 1 of 3");
    assert_eq!(v.rows[0].comment.as_deref(), Some("Header only"));
    assert_eq!(v.rows[1].comment.as_deref(), Some("No EUR data"));
}
#[test]
fn later_attempts_do_not_show_an_out_of_three_limit() {
    let mut s = ended(SessionEnd::Verdict(Verdict::NeedsFixes));
    if let SessionState::Ended(end) = &mut s.state {
        end.attempt = Some(crate::attempts::Attempt(4));
    }
    assert!(for_snapshot(&s).unwrap().tally.ends_with("attempt 4"));
}
#[test]
fn partly_verified_shows_passed_and_skipped_counts() {
    let mut s = ended(SessionEnd::Verdict(Verdict::PartlyVerified));
    s.checks.push(CheckStatus {
        id: CheckId(2),
        text: "SSO".into(),
        added: false,
        mark: Mark::Outcome {
            outcome: CheckOutcome::Skipped,
            comment: "No identity provider".into(),
            at: Millis(0),
        },
    });
    let v = for_snapshot(&s).unwrap();
    assert_eq!(v.title, "Partly verified");
    assert_eq!(v.tally, "1 passed, 1 skipped");
}
#[test]
fn incomplete_shows_the_reason_and_unmarked_rows() {
    let mut s = ended(SessionEnd::Incomplete(IncompleteReason::Idle {
        limit: std::time::Duration::from_secs(300),
    }));
    s.checks.push(CheckStatus {
        id: CheckId(2),
        text: "PDF".into(),
        added: false,
        mark: Mark::Unmarked,
    });
    let v = for_snapshot(&s).unwrap();
    assert_eq!(v.title, "Incomplete");
    assert_eq!(v.subtitle, "No tool call for 5 min, session ended");
    assert_eq!(v.tally, "1 of 2 checked");
    assert_eq!(v.rows[1].tick, super::super::footer_view::Tick::Unmarked);
}
#[test]
fn detached_dirty_revision_has_no_attempt() {
    let mut s = ended(SessionEnd::Verdict(Verdict::NeedsFixes));
    if let SessionState::Ended(e) = &mut s.state {
        e.revision.branch = None;
        e.revision.dirty = true;
        e.attempt = None;
    }
    let v = for_snapshot(&s).unwrap();
    assert_eq!(
        v.subtitle,
        "Checked detached HEAD at e4f09b2 + uncommitted changes"
    );
    assert!(!v.tally.contains("attempt"));
}
