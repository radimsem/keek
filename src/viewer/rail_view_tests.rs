use super::*;
use crate::{
    checklist::{Mark, SessionEnd, Verdict},
    ipc::{CheckStatus, ControlState, HostShortcuts, LastAction, SessionState, TakeoverPhase},
    timeline::{CheckId, CheckOutcome, Millis},
    viewer::footer_view::Tick,
};

fn at(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(seconds, 0).unwrap()
}

fn info(slot: usize) -> crate::viewer::SessionInfo {
    let placement = crate::slots::geometry(
        crate::slots::Slot {
            x: 24,
            y: 50,
            width: 2822,
            height: 1298,
        },
        None,
    );
    crate::viewer::SessionInfo {
        name: "billing-12-00-00".parse().unwrap(),
        project: "billing".into(),
        branch: Some("fix/17-invite-role".into()),
        goal: "An invited member gets the Editor role".into(),
        opened: at(0),
        slot: crate::viewer::SlotId(slot),
        placement,
        monitor: "DP-1".into(),
        spot: crate::viewer::CardSpot::Rail(crate::slots::Slot {
            x: 2856,
            y: 50,
            width: 560,
            height: 384,
        }),
    }
}

fn check(id: u32, text: &str, outcome: Option<CheckOutcome>) -> CheckStatus {
    CheckStatus {
        id: CheckId(id),
        text: text.into(),
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

fn working() -> SessionSnapshot {
    SessionSnapshot {
        control: ControlState::Verifier,
        state: SessionState::Open,
        checks: vec![
            check(
                1,
                "Team page lists current members",
                Some(CheckOutcome::Passed),
            ),
            check(2, "Invite form accepts an email and a role", None),
            check(3, "The new member shows as Editor", None),
        ],
        reminded: false,
        last_action: Some(LastAction {
            tool: "click".into(),
            time_ms: Millis(38_000),
        }),
    }
}

#[test]
fn a_rail_card_shows_project_branch_elapsed_state_ticks_and_its_shortcut() {
    let view = for_rail(
        &info(1),
        &working(),
        at(42),
        Some(at(39)),
        false,
        bound(&[1, 2]),
    );
    assert_eq!(view.project, "billing");
    assert_eq!(view.branch, "fix/17-invite-role");
    assert_eq!(view.elapsed, "0:42");
    assert_eq!(view.needs_you, None);
    assert_eq!(view.line, "click · 0:04 ago");
    let checked = view.checked.unwrap();
    assert_eq!(checked.count, "1 of 3");
    assert_eq!(
        checked.ticks,
        vec![Tick::Passed, Tick::InProgress, Tick::Unmarked]
    );
    assert_eq!(view.keys, Some(["Super".to_owned(), "F2".to_owned()]));
    assert_eq!(view.still.as_deref(), Some("Still · 3 s ago"));
    assert_eq!(view.accessible, "Show billing");
    assert!(!view.locked);
}

#[test]
fn a_rail_card_without_a_still_yet_shows_no_age() {
    let view = for_rail(
        &info(1),
        &working(),
        at(42),
        None,
        false,
        ShowKeys::default(),
    );
    assert_eq!(view.still, None);
}

#[test]
fn an_old_still_says_its_age_in_minutes() {
    let view = for_rail(
        &info(1),
        &working(),
        at(200),
        Some(at(70)),
        false,
        ShowKeys::default(),
    );
    assert_eq!(view.still.as_deref(), Some("Still · 2 min ago"));
}

#[test]
fn only_the_first_four_sessions_have_a_shortcut() {
    let keys = bound(&[1, 2, 3, 4, 5]);
    assert_eq!(
        for_rail(&info(3), &working(), at(42), None, false, keys).keys,
        Some(["Super".to_owned(), "F4".to_owned()])
    );
    assert_eq!(
        for_rail(&info(4), &working(), at(42), None, false, keys).keys,
        None
    );
}

fn bound(numbers: &[u8]) -> ShowKeys {
    let binds: Vec<_> = numbers
        .iter()
        .map(|n| serde_json::json!({"submap": "", "key": format!("F{n}"), "modmask": 64}))
        .collect();
    ShowKeys::from_binds(&serde_json::Value::from(binds))
}

#[test]
fn a_rail_card_shows_its_shortcut_only_once_its_super_function_key_is_bound() {
    for (keys, expected) in [
        (ShowKeys::default(), None),
        (bound(&[1, 3]), None),
        (bound(&[2]), Some(["Super".to_owned(), "F2".to_owned()])),
    ] {
        assert_eq!(
            for_rail(&info(1), &working(), at(42), None, false, keys).keys,
            expected
        );
    }
}

#[test]
fn only_a_super_function_key_bind_outside_any_submap_counts_as_bound() {
    for (submap, key, mask, expected) in [
        ("", "F2", 64, true),
        ("", "f2", 65, true),
        ("keek", "F2", 64, false),
        ("", "F2", 0, false),
        ("", "2", 64, false),
        ("", "F12", 64, false),
    ] {
        let binds = serde_json::json!([{"submap": submap, "key": key, "modmask": mask}]);
        assert_eq!(
            for_rail(
                &info(1),
                &working(),
                at(42),
                None,
                false,
                ShowKeys::from_binds(&binds)
            )
            .keys
            .is_some(),
            expected,
            "{submap:?} {key} {mask}"
        );
    }
}

#[test]
fn a_help_request_makes_the_card_say_needs_you_with_its_reason_and_drops_the_ring() {
    let mut snapshot = working();
    snapshot.control = ControlState::HelpRequest {
        reason: "Recovery key for the export step".into(),
        start_ms: Millis(30_000),
    };
    let view = for_rail(
        &info(1),
        &snapshot,
        at(42),
        None,
        false,
        ShowKeys::default(),
    );
    assert_eq!(
        view.needs_you.as_deref(),
        Some("Recovery key for the export step")
    );
    assert_eq!(
        view.checked.unwrap().ticks,
        vec![Tick::Passed, Tick::Unmarked, Tick::Unmarked]
    );
}

#[test]
fn hovering_shows_the_goal_and_each_check_with_the_one_being_checked_tagged() {
    let view = for_rail(
        &info(1),
        &working(),
        at(42),
        None,
        false,
        ShowKeys::default(),
    );
    assert_eq!(view.hover.goal, "An invited member gets the Editor role");
    assert_eq!(
        view.hover.checks,
        vec![
            HoverCheck {
                text: "Team page lists current members".into(),
                mark: HoverMark::Passed
            },
            HoverCheck {
                text: "Invite form accepts an email and a role".into(),
                mark: HoverMark::Checking
            },
            HoverCheck {
                text: "The new member shows as Editor".into(),
                mark: HoverMark::Unmarked
            },
        ]
    );
    assert_eq!(view.hover.hint, "Click to show this session.");
}

#[test]
fn hovering_shows_failed_and_skipped_outcomes() {
    let mut snapshot = working();
    snapshot.checks[1].mark = Mark::Outcome {
        outcome: CheckOutcome::Failed,
        comment: "No role field".into(),
        at: Millis(0),
    };
    snapshot.checks[2].mark = Mark::Outcome {
        outcome: CheckOutcome::Skipped,
        comment: String::new(),
        at: Millis(0),
    };
    let marks: Vec<_> = for_rail(
        &info(1),
        &snapshot,
        at(42),
        None,
        false,
        ShowKeys::default(),
    )
    .hover
    .checks
    .into_iter()
    .map(|check| check.mark)
    .collect();
    assert_eq!(
        marks,
        vec![HoverMark::Passed, HoverMark::Failed, HoverMark::Skipped]
    );
}

#[test]
fn an_ended_rail_card_shows_its_verdict() {
    let snapshot =
        crate::viewer::verdict_view::tests::ended(SessionEnd::Verdict(Verdict::NeedsFixes));
    let view = for_rail(
        &info(1),
        &snapshot,
        at(42),
        None,
        false,
        ShowKeys::default(),
    );
    assert_eq!(view.verdict, Some(StateLabel::NeedsFixes));
    assert_eq!(view.line, "Needs fixes");
    assert!(
        view.checked
            .is_none_or(|checked| !checked.ticks.contains(&Tick::InProgress))
    );
}

#[test]
fn a_running_or_paused_takeover_locks_the_rail() {
    let mut running = working();
    running.control = ControlState::Takeover {
        start_ms: Millis(1_000),
        shortcuts: HostShortcuts::Captured,
        phase: TakeoverPhase::Running,
    };
    let mut paused = running.clone();
    paused.control = ControlState::Takeover {
        start_ms: Millis(1_000),
        shortcuts: HostShortcuts::Captured,
        phase: TakeoverPhase::Paused {
            since_ms: Millis(2_000),
        },
    };
    let mut helped = working();
    helped.control = ControlState::HelpRequest {
        reason: "2FA".into(),
        start_ms: Millis(0),
    };
    assert!(rail_locked([&working(), &running]));
    assert!(rail_locked([&paused, &working()]));
    assert!(!rail_locked([&working(), &helped]));
    assert!(
        for_rail(
            &info(1),
            &working(),
            at(42),
            None,
            true,
            ShowKeys::default()
        )
        .locked
    );
}

fn card(x: i32, y: i32) -> crate::slots::Slot {
    crate::slots::Slot {
        x,
        y,
        width: 560,
        height: 384,
    }
}

#[test]
fn a_monitor_at_least_1920_wide_has_a_rail_column_and_a_narrower_one_a_rail_row() {
    use crate::slots::RailShape;
    assert_eq!(RailShape::for_width(1920), RailShape::Column);
    assert_eq!(RailShape::for_width(1919), RailShape::Row);
}

#[test]
fn the_hover_panel_opens_left_of_a_rail_column_card_and_above_a_rail_row_card() {
    use crate::slots::RailShape;
    assert_eq!(
        hover_spot(card(2856, 445), RailShape::Column),
        HoverSpot::LeftOf { x: 2404, top: 445 }
    );
    assert_eq!(
        hover_spot(card(24, 688), RailShape::Row),
        HoverSpot::Above { x: 24, bottom: 676 }
    );
}

#[test]
fn the_locked_note_sits_under_a_rail_column_and_inside_the_top_of_a_rail_row() {
    use crate::slots::RailShape;
    let column = [card(2856, 50), card(2856, 445), card(2856, 840)];
    assert_eq!(
        locked_note_spot(&column, RailShape::Column),
        Some(NoteSpot {
            x: 2856,
            top: 1240,
            width: 560
        })
    );
    let row = [card(24, 688), card(595, 688)];
    assert_eq!(
        locked_note_spot(&row, RailShape::Row),
        Some(NoteSpot {
            x: 32,
            top: 696,
            width: 1115
        })
    );
    assert_eq!(
        locked_note_spot(&[card(24, 688)], RailShape::Row),
        Some(NoteSpot {
            x: 32,
            top: 696,
            width: 544
        })
    );
    assert_eq!(locked_note_spot(&[], RailShape::Column), None);
}
