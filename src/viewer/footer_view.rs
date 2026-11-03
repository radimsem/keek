use crate::{
    checklist::Mark,
    ipc::{SessionSnapshot, SessionState},
    timeline::CheckOutcome,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    Over,
    Away,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLabel {
    Opening,
    LastAction,
    ViewOnly,
    Closing,
}
impl StateLabel {
    pub fn text(self) -> &'static str {
        match self {
            Self::Opening => "Opening",
            Self::LastAction => "Last action",
            Self::ViewOnly => "View only",
            Self::Closing => "Closing",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    Passed,
    Failed,
    Skipped,
    Unmarked,
}
impl Tick {
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Unmarked => "unmarked",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionColour {
    Cyan,
    Violet,
}
impl SessionColour {
    pub fn for_slot(slot: super::SlotId) -> Self {
        match slot.0 % 2 {
            0 => Self::Cyan,
            _ => Self::Violet,
        }
    }
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Cyan => "cyan",
            Self::Violet => "violet",
        }
    }
    pub fn color(self) -> &'static str {
        match self {
            Self::Cyan => "#4ccce0",
            Self::Violet => "#a77bf3",
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct CheckedView {
    pub count: String,
    pub ticks: Vec<Tick>,
    pub overflow: usize,
    pub accessible: String,
}
#[derive(Debug, PartialEq, Eq)]
pub struct FooterView {
    pub project: String,
    pub branch: String,
    pub goal: String,
    pub label: StateLabel,
    pub detail: String,
    pub elapsed_label: &'static str,
    pub elapsed: String,
    pub checked: Option<CheckedView>,
}
pub fn duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}
pub fn action_words(tool: &str) -> &str {
    match tool {
        "type_text" | "type" => "type text",
        "press_key" | "press_keys" | "key" | "hotkey" => "press keys",
        "click" => "click",
        "double_click" => "double click",
        "right_click" => "right click",
        "set_value" => "set value",
        "scroll" => "scroll",
        "move" | "move_mouse" => "move pointer",
        "drag" => "drag",
        "screenshot" => "take screenshot",
        _ => "use desktop",
    }
}
pub fn for_session(
    info: &super::SessionInfo,
    snapshot: &SessionSnapshot,
    now: chrono::DateTime<chrono::Utc>,
    hover: Hover,
    frozen: Option<u64>,
) -> FooterView {
    let seconds = frozen
        .unwrap_or_else(|| now.signed_duration_since(info.opened).num_seconds().max(0) as u64);
    let (label, detail, elapsed_label) = match snapshot.state {
        SessionState::Opening => (
            StateLabel::Opening,
            "Preparing the desktop.".into(),
            "Elapsed",
        ),
        SessionState::Closing => (StateLabel::Closing, "Ending the session.".into(), "Ran"),
        SessionState::Open if hover == Hover::Over => (
            StateLabel::ViewOnly,
            "Use Take control to interact.".into(),
            "Elapsed",
        ),
        SessionState::Open => (
            StateLabel::LastAction,
            snapshot.last_action.as_ref().map_or_else(
                || "Waiting for the Verifier.".into(),
                |a| {
                    format!(
                        "{} · {} ago",
                        action_words(&a.tool),
                        duration(seconds.saturating_sub(a.time_ms.0 / 1000))
                    )
                },
            ),
            "Elapsed",
        ),
    };
    let mut counts = [0; 4];
    let mut ticks = Vec::new();
    for check in &snapshot.checks {
        let (index, tick) = match &check.mark {
            Mark::Unmarked => (3, Tick::Unmarked),
            Mark::Outcome { outcome, .. } => match outcome {
                CheckOutcome::Passed => (0, Tick::Passed),
                CheckOutcome::Failed => (1, Tick::Failed),
                CheckOutcome::Skipped => (2, Tick::Skipped),
            },
        };
        counts[index] += 1;
        if ticks.len() < 8 {
            ticks.push(tick);
        }
    }
    let checked = (!snapshot.checks.is_empty()).then(|| CheckedView {
        count: format!(
            "{} of {}",
            counts[0] + counts[1] + counts[2],
            snapshot.checks.len()
        ),
        ticks,
        overflow: snapshot.checks.len().saturating_sub(8),
        accessible: format!(
            "{} passed, {} failed, {} skipped, {} not checked yet",
            counts[0], counts[1], counts[2], counts[3]
        ),
    });
    FooterView {
        project: info.project.clone(),
        branch: info
            .branch
            .clone()
            .unwrap_or_else(|| "detached HEAD".into()),
        goal: info.goal.clone(),
        label,
        detail,
        elapsed_label,
        elapsed: duration(seconds),
        checked,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn render(snapshot: &SessionSnapshot, seconds: u64, hover: bool) -> FooterView {
        for_session(
            &info(),
            snapshot,
            chrono::DateTime::from_timestamp(seconds as i64, 0).unwrap(),
            if hover { Hover::Over } else { Hover::Away },
            None,
        )
    }
    #[test]
    fn slot_colours_repeat_after_two_sessions() {
        assert_eq!(
            SessionColour::for_slot(super::super::SlotId(2)),
            SessionColour::Cyan
        );
        assert_eq!(
            SessionColour::for_slot(super::super::SlotId(3)),
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
            state,
            checks: vec![],
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
    fn info() -> super::super::SessionInfo {
        super::super::SessionInfo {
            name: "app_12-00-00".parse().unwrap(),
            project: "app".into(),
            branch: None,
            goal: "verify login".into(),
            opened: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            slot: super::super::SlotId(1),
            placement: crate::slots::Slot {
                x: 0,
                y: 0,
                width: 1000,
                height: 800,
            },
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
