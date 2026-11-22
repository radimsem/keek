use super::*;
mod tests {
    use super::*;
    #[test]
    fn control_labels_map_to_their_css_classes() {
        assert_eq!(StateLabel::NeedsYou.css_class(), Some("needs-you"));
        assert_eq!(StateLabel::Takeover.css_class(), Some("takeover"));
        assert_eq!(StateLabel::HandBack.css_class(), Some("hand-back"));
        for label in [
            StateLabel::Opening,
            StateLabel::LastAction,
            StateLabel::ViewOnly,
            StateLabel::Closing,
        ] {
            assert_eq!(label.css_class(), None);
        }
    }
    pub(super) fn render(snapshot: &SessionSnapshot, seconds: u64, hover: bool) -> FooterView {
        for_session(
            &info(),
            snapshot,
            chrono::DateTime::from_timestamp(seconds as i64, 0).unwrap(),
            if hover { Hover::Over } else { Hover::Away },
            None,
        )
    }
    #[test]
    fn a_help_request_shows_its_reason_wait_and_orange_control() {
        let mut s = snapshot(SessionState::Open);
        s.control = crate::ipc::ControlState::HelpRequest {
            reason: "2FA code".into(),
            start_ms: crate::timeline::Millis(1000),
        };
        let view = render(&s, 25, true);
        assert_eq!(view.label.text(), "Needs you");
        assert_eq!(view.detail, "2FA code · waiting 0:24");
        assert_eq!(view.button, Some(FooterButton::TakeControl));
    }
    #[test]
    fn takeover_suppresses_the_hover_hint_and_shows_captured_shortcuts() {
        let mut s = snapshot(SessionState::Open);
        s.control = crate::ipc::ControlState::Takeover {
            start_ms: crate::timeline::Millis(1000),
            shortcuts: crate::ipc::HostShortcuts::Captured,
        };
        let view = render(&s, 19, true);
        assert_eq!(view.label.text(), "You are in control");
        assert_eq!(view.detail, "0:18 · host shortcuts go to the session");
        assert_eq!(view.button, Some(FooterButton::Done));
        assert!(view.keycaps);
    }
    #[test]
    fn takeover_warns_when_the_submap_is_missing() {
        let mut s = snapshot(SessionState::Open);
        s.control = crate::ipc::ControlState::Takeover {
            start_ms: crate::timeline::Millis(1000),
            shortcuts: crate::ipc::HostShortcuts::Live,
        };
        let view = render(&s, 19, false);
        assert_eq!(
            view.detail,
            "0:18 · host shortcuts stay live (no keek submap)"
        );
        assert!(!view.keycaps);
        assert_eq!(view.label.text(), "You are in control");
        assert_eq!(view.button, Some(FooterButton::Done));
    }
    #[test]
    fn hand_back_stays_visible_and_adds_only_later_actions() {
        let mut s = snapshot(SessionState::Open);
        s.control = crate::ipc::ControlState::HandBack {
            end_ms: crate::timeline::Millis(43000),
            length: crate::timeline::Millis(42000),
        };
        let view = render(&s, 45, false);
        assert_eq!(view.label.text(), "Back to the Verifier");
        assert_eq!(view.detail, "You were in control for 0:42");
        assert_eq!(view.button, Some(FooterButton::TakeControl));
        assert_eq!(render(&s, 45, true).label.text(), "View only");
        for time in [42000, 43000] {
            s.last_action = Some(crate::ipc::LastAction {
                tool: "click".into(),
                time_ms: crate::timeline::Millis(time),
            });
            assert_eq!(render(&s, 45, false).detail, "You were in control for 0:42");
        }
        s.last_action = Some(crate::ipc::LastAction {
            tool: "click".into(),
            time_ms: crate::timeline::Millis(44000),
        });
        assert_eq!(
            render(&s, 45, false).detail,
            "You were in control for 0:42 · click · 0:01 ago"
        );
    }
    #[test]
    fn non_open_sessions_never_offer_a_control_button() {
        for state in [SessionState::Opening, SessionState::Closing] {
            let mut s = snapshot(state);
            s.control = crate::ipc::ControlState::Takeover {
                start_ms: crate::timeline::Millis(0),
                shortcuts: crate::ipc::HostShortcuts::Live,
            };
            assert_eq!(render(&s, 0, false).button, None);
        }
    }
    #[test]
    fn slot_colours_repeat_after_two_sessions() {
        assert_eq!(
            SessionColour::for_slot(super::super::super::SlotId(2)),
            SessionColour::Cyan
        );
        assert_eq!(
            SessionColour::for_slot(super::super::super::SlotId(3)),
            SessionColour::Violet
        );
    }
    #[test]
    fn no_checks_hide_the_checked_column() {
        assert!(
            render(&snapshot(SessionState::Open), 0, false)
                .checked
                .is_none()
        );
    }
    fn snapshot(state: SessionState) -> SessionSnapshot {
        SessionSnapshot {
            control: crate::ipc::ControlState::Verifier,
            state,
            checks: vec![],
            reminded: false,
            last_action: None,
        }
    }
    #[test]
    fn driver_actions_use_plain_words_and_unknown_actions_use_fallback() {
        assert_eq!(action_words("right_click"), "right click");
        assert_eq!(action_words("hotkey"), "press keys");
        assert_eq!(action_words("set_value"), "set value");
        assert_eq!(action_words("unknown"), "use desktop");
    }
    #[test]
    fn clocks_use_minutes_and_hours() {
        assert_eq!(duration(65), "1:05");
        assert_eq!(duration(3661), "1:01:01");
    }
    #[test]
    fn opening_has_preparing_copy() {
        let v = render(&snapshot(SessionState::Opening), 0, false);
        assert_eq!(
            (v.label.text(), v.detail.as_str()),
            ("Opening", "Preparing the desktop.")
        );
    }
    #[test]
    fn waiting_has_verifier_copy() {
        let v = render(&snapshot(SessionState::Open), 0, false);
        assert_eq!(
            (v.label.text(), v.detail.as_str()),
            ("Last action", "Waiting for the Verifier.")
        );
    }
    #[test]
    fn hover_has_view_only_copy() {
        let v = render(&snapshot(SessionState::Open), 0, true);
        assert_eq!(
            (v.label.text(), v.detail.as_str()),
            ("View only", "Use Take control to interact.")
        );
    }
    #[test]
    fn closing_has_ran_copy() {
        let v = render(&snapshot(SessionState::Closing), 65, true);
        assert_eq!((v.label.text(), v.elapsed_label), ("Closing", "Ran"));
    }
    fn info() -> super::super::super::SessionInfo {
        super::super::super::SessionInfo {
            name: "app_12-00-00".parse().unwrap(),
            project: "app".into(),
            branch: None,
            goal: "verify login".into(),
            opened: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            slot: super::super::super::SlotId(1),
            placement: crate::slots::geometry(
                crate::slots::Slot {
                    x: 0,
                    y: 0,
                    width: 1000,
                    height: 800,
                },
                None,
            ),
            monitor: "DP-1".into(),
        }
    }
    #[test]
    fn session_identity_uses_detached_branch_and_slot_color() {
        let v = for_session(
            &info(),
            &snapshot(SessionState::Open),
            chrono::DateTime::from_timestamp(65, 0).unwrap(),
            Hover::Over,
            None,
        );
        assert_eq!(
            (v.project.as_str(), v.branch.as_str(), v.goal.as_str()),
            ("app", "detached HEAD", "verify login")
        );
        assert_eq!(SessionColour::for_slot(info().slot), SessionColour::Violet);
        assert_eq!(v.elapsed, "1:05");
    }
    #[test]
    fn closing_clock_keeps_its_frozen_duration() {
        let v = for_session(
            &info(),
            &snapshot(SessionState::Closing),
            chrono::DateTime::from_timestamp(999, 0).unwrap(),
            Hover::Away,
            Some(65),
        );
        assert_eq!(v.elapsed, "1:05");
    }
    #[test]
    fn checklist_accessibility_names_all_marked_outcomes() {
        let mut s = snapshot(SessionState::Open);
        for (n, outcome) in [
            CheckOutcome::Passed,
            CheckOutcome::Passed,
            CheckOutcome::Failed,
            CheckOutcome::Skipped,
        ]
        .into_iter()
        .enumerate()
        {
            s.checks.push(crate::ipc::CheckStatus {
                added: false,
                id: crate::timeline::CheckId(n as u32),
                text: "check".into(),
                mark: Mark::Outcome {
                    outcome,
                    comment: String::new(),
                    at: crate::timeline::Millis(0),
                },
            });
        }
        assert_eq!(
            render(&s, 0, false).checked.unwrap().accessible,
            "2 passed, 1 failed, 1 skipped, 0 not checked yet"
        );
    }
    #[test]
    fn actions_show_words_and_age_without_arguments() {
        let mut s = snapshot(SessionState::Open);
        s.last_action = Some(crate::ipc::LastAction {
            tool: "type_text".into(),
            time_ms: crate::timeline::Millis(2000),
        });
        assert_eq!(render(&s, 5, false).detail, "type text · 0:03 ago");
    }
    #[test]
    fn checklist_caps_ticks_and_names_every_outcome() {
        let mut s = snapshot(SessionState::Open);
        for n in 0..10 {
            s.checks.push(crate::ipc::CheckStatus {
                added: false,
                id: crate::timeline::CheckId(n),
                text: "check".into(),
                mark: if n == 0 {
                    Mark::Outcome {
                        outcome: CheckOutcome::Passed,
                        comment: String::new(),
                        at: crate::timeline::Millis(0),
                    }
                } else {
                    Mark::Unmarked
                },
            });
        }
        let v = render(&s, 0, false);
        let checked = v.checked.unwrap();
        assert_eq!(checked.count, "1 of 10");
        assert_eq!(checked.ticks.len(), 8);
        assert_eq!(checked.overflow, 2);
        assert_eq!(
            checked.accessible,
            "1 passed, 0 failed, 0 skipped, 9 not checked yet"
        );
    }
}

mod ended_tests {
    use super::*;
    use crate::{
        checklist::{IncompleteReason, SessionEnd, Verdict},
        ipc::{Percent, SessionState},
        timeline::Millis,
    };
    fn check_footer(s: &SessionSnapshot, label: StateLabel, detail: &str) {
        let view = tests::render(s, 999, true);
        assert_eq!(view.label, label);
        assert_eq!(view.detail, detail);
        assert_eq!(view.elapsed, "3:02");
        assert_eq!(view.elapsed_label, "Ran");
        assert_eq!(
            view.buttons,
            vec![
                FooterButton::OpenVideo,
                FooterButton::OpenFolder,
                FooterButton::Dismiss
            ]
        );
    }
    fn skipped(s: &mut SessionSnapshot) {
        s.checks.push(crate::ipc::CheckStatus {
            id: crate::timeline::CheckId(2),
            text: "SSO".into(),
            added: false,
            mark: Mark::Outcome {
                outcome: CheckOutcome::Skipped,
                comment: "No provider".into(),
                at: Millis(0),
            },
        });
    }
    #[test]
    fn pass_footer_shows_video_and_actions() {
        let s = super::super::super::verdict_view::tests::ended(SessionEnd::Verdict(Verdict::Pass));
        check_footer(&s, StateLabel::Pass, "0:38 video · 14 actions");
        let mut s = s;
        if let SessionState::Ended(end) = &mut s.state {
            end.actions = 1;
        }
        check_footer(&s, StateLabel::Pass, "0:38 video · 1 action");
    }
    #[test]
    fn needs_fixes_footer_shows_failed_and_skipped_checks() {
        let mut s = super::super::super::verdict_view::tests::ended(SessionEnd::Verdict(
            Verdict::NeedsFixes,
        ));
        s.checks[0].mark = Mark::Outcome {
            outcome: CheckOutcome::Failed,
            comment: "Missing lines".into(),
            at: Millis(0),
        };
        skipped(&mut s);
        check_footer(
            &s,
            StateLabel::NeedsFixes,
            "1 check failed, 1 skipped · attempt 1 of 3",
        );
    }
    #[test]
    fn partly_verified_footer_shows_skipped_checks() {
        let mut s = super::super::super::verdict_view::tests::ended(SessionEnd::Verdict(
            Verdict::PartlyVerified,
        ));
        skipped(&mut s);
        check_footer(&s, StateLabel::PartlyVerified, "1 check skipped");
    }
    #[test]
    fn incomplete_footer_shows_why_the_session_ended() {
        let s = super::super::super::verdict_view::tests::ended(SessionEnd::Incomplete(
            IncompleteReason::DesktopStopped,
        ));
        check_footer(&s, StateLabel::Incomplete, "Desktop stopped");
    }
    #[test]
    fn only_live_session_states_need_a_clock_tick() {
        let mut s =
            super::super::super::verdict_view::tests::ended(SessionEnd::Verdict(Verdict::Pass));
        assert!(!s.state.ticks());
        s.state = SessionState::Rendering {
            percent: Percent(1),
            frame: None,
            elapsed: Millis(0),
        };
        assert!(!s.state.ticks());
        for state in [
            SessionState::Opening,
            SessionState::Open,
            SessionState::Closing,
        ] {
            assert!(state.ticks());
        }
    }
    #[test]
    fn missing_evidence_hides_open_buttons() {
        let mut s =
            super::super::super::verdict_view::tests::ended(SessionEnd::Verdict(Verdict::Pass));
        if let SessionState::Ended(e) = &mut s.state {
            e.verification = None;
            e.folder = None;
        }
        assert_eq!(
            tests::render(&s, 999, true).buttons,
            vec![FooterButton::Dismiss]
        );
        assert_eq!(tests::render(&s, 999, true).detail, "14 actions");
    }
    #[test]
    fn later_attempts_do_not_show_an_out_of_three_limit() {
        let mut s = super::super::super::verdict_view::tests::ended(SessionEnd::Verdict(
            Verdict::NeedsFixes,
        ));
        if let SessionState::Ended(end) = &mut s.state {
            end.attempt = Some(crate::attempts::Attempt(4));
        }
        assert!(tests::render(&s, 999, true).detail.ends_with("attempt 4"));
    }
    #[test]
    fn rendering_footer_shows_percentage_and_frozen_duration() {
        let mut s =
            super::super::super::verdict_view::tests::ended(SessionEnd::Verdict(Verdict::Pass));
        s.state = SessionState::Rendering {
            percent: Percent(64),
            frame: None,
            elapsed: Millis(65000),
        };
        let v = tests::render(&s, 999, true);
        assert_eq!(v.label.text(), "Rendering");
        assert_eq!(v.detail, "64%");
        assert_eq!(v.elapsed, "1:05");
        assert!(v.buttons.is_empty());
    }
}

mod live_checks {
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
        };
        assert_eq!(tests::render(&s, 0, false).label, StateLabel::Takeover);
    }
}
