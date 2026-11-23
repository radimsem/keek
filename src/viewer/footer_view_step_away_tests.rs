use super::*;
use crate::ipc::{ControlState, HostShortcuts, TakeoverPhase};
use crate::timeline::Millis;

fn info() -> super::super::SessionInfo {
    super::super::SessionInfo {
        name: "app_12-00-00".parse().unwrap(),
        project: "app".into(),
        branch: None,
        goal: "verify login".into(),
        opened: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        slot: super::super::SlotId(1),
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

fn snapshot(control: ControlState) -> SessionSnapshot {
    SessionSnapshot {
        control,
        state: SessionState::Open,
        checks: vec![],
        last_action: None,
        reminded: false,
    }
}

fn takeover(shortcuts: HostShortcuts, phase: TakeoverPhase) -> SessionSnapshot {
    snapshot(ControlState::Takeover {
        start_ms: Millis(1000),
        shortcuts,
        phase,
    })
}

fn paused() -> SessionSnapshot {
    takeover(
        HostShortcuts::Captured,
        TakeoverPhase::Paused {
            since_ms: Millis(20_000),
        },
    )
}

fn render(snapshot: &SessionSnapshot, seconds: i64, hover: Hover) -> FooterView {
    for_session(
        &info(),
        snapshot,
        chrono::DateTime::from_timestamp(seconds, 0).unwrap(),
        hover,
        None,
    )
}

#[test]
fn a_running_takeover_offers_step_away_beside_done_with_their_shortcuts() {
    let view = render(
        &takeover(HostShortcuts::Captured, TakeoverPhase::Running),
        19,
        Hover::Away,
    );
    assert_eq!(view.detail, "0:18 · host shortcuts go to the session");
    assert_eq!(view.buttons, [FooterButton::StepAway, FooterButton::Done]);
    assert_eq!(
        view.shortcuts,
        [
            (FooterButton::StepAway, Shortcut::SuperTab),
            (FooterButton::Done, Shortcut::SuperEsc)
        ]
    );
    assert_eq!(FooterButton::StepAway.text(), "Step away");
    assert_eq!(Shortcut::SuperTab.keys(), ["Super", "Tab"]);
    assert_eq!(Shortcut::SuperEsc.keys(), ["Super", "Esc"]);
    assert_eq!(view.takeover, Some(Takeover::Running));
}

#[test]
fn a_snippet_without_super_tab_says_the_shortcut_is_missing_and_keeps_the_button() {
    let view = render(
        &takeover(
            HostShortcuts::CapturedWithoutStepAway,
            TakeoverPhase::Running,
        ),
        19,
        Hover::Away,
    );
    assert_eq!(
        view.detail,
        "0:18 · Step away shortcut missing from the keek submap"
    );
    assert_eq!(view.buttons, [FooterButton::StepAway, FooterButton::Done]);
    assert_eq!(view.shortcuts, [(FooterButton::Done, Shortcut::SuperEsc)]);
}

#[test]
fn without_the_keek_submap_step_away_still_shows_without_shortcuts() {
    let view = render(
        &takeover(HostShortcuts::Live, TakeoverPhase::Running),
        19,
        Hover::Away,
    );
    assert_eq!(view.buttons, [FooterButton::StepAway, FooterButton::Done]);
    assert!(view.shortcuts.is_empty());
}

#[test]
fn a_paused_takeover_reads_takeover_paused_over_the_time_away() {
    for hover in [Hover::Away, Hover::Over] {
        let view = render(&paused(), 61, hover);
        assert_eq!(view.label, StateLabel::TakeoverPaused);
        assert_eq!(view.label.text(), "Takeover paused");
        assert_eq!(view.label.css_class(), Some("paused"));
        assert_eq!(view.detail, "0:41 away · your shortcuts work again");
        assert_eq!(view.buttons, [FooterButton::Resume, FooterButton::Done]);
        assert!(
            view.shortcuts.is_empty(),
            "Super+Esc does nothing while paused"
        );
        assert_eq!(view.takeover, Some(Takeover::Paused));
    }
    assert_eq!(FooterButton::Resume.text(), "Resume");
}

#[test]
fn only_a_takeover_reports_a_takeover_phase() {
    for control in [
        ControlState::Verifier,
        ControlState::HelpRequest {
            reason: "2FA code".into(),
            start_ms: Millis(0),
        },
    ] {
        assert_eq!(render(&snapshot(control), 5, Hover::Away).takeover, None);
    }
}

#[test]
fn a_click_on_the_card_body_resumes_only_a_paused_takeover() {
    assert_eq!(body_click(&paused()), Some(FooterButton::Resume));
    for other in [
        takeover(HostShortcuts::Captured, TakeoverPhase::Running),
        snapshot(ControlState::Verifier),
        snapshot(ControlState::HelpRequest {
            reason: "2FA code".into(),
            start_ms: Millis(0),
        }),
    ] {
        assert_eq!(body_click(&other), None);
    }
    let mut closing = paused();
    closing.state = SessionState::Closing;
    assert_eq!(body_click(&closing), None);
}

#[test]
fn the_paused_panel_is_never_wider_than_the_session_window() {
    for window in [(390, 844), (1280, 800), (2822, 1298)] {
        let geometry = crate::slots::geometry(
            crate::slots::Slot {
                x: 24,
                y: 50,
                width: 2822,
                height: 1298,
            },
            Some(window),
        );
        assert!(super::super::paused_card::panel_width(geometry) <= geometry.window.width);
    }
}
