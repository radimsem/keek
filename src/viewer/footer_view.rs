use crate::{
    checklist::Mark,
    ipc::{SessionSnapshot, SessionState},
    reminder::THRESHOLD,
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
    Reminded,
    ViewOnly,
    Closing,
    NeedsYou,
    Takeover,
    HandBack,
    Rendering,
    Pass,
    NeedsFixes,
    PartlyVerified,
    Incomplete,
}
impl StateLabel {
    pub fn css_class(self) -> Option<&'static str> {
        match self {
            Self::Pass => Some("passed"),
            Self::NeedsFixes => Some("failed"),
            Self::PartlyVerified => Some("skipped"),
            Self::Rendering | Self::Incomplete => None,
            Self::NeedsYou => Some("needs-you"),
            Self::Takeover => Some("takeover"),
            Self::HandBack => Some("hand-back"),
            Self::Opening | Self::LastAction | Self::Reminded | Self::ViewOnly | Self::Closing => {
                None
            }
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Self::Rendering => "Rendering",
            Self::Pass => "Pass",
            Self::NeedsFixes => "Needs fixes",
            Self::PartlyVerified => "Partly verified",
            Self::Incomplete => "Incomplete",
            Self::Opening => "Opening",
            Self::LastAction => "Last action",
            Self::Reminded => "Reminded the Verifier to mark Checks",
            Self::ViewOnly => "View only",
            Self::Closing => "Closing",
            Self::NeedsYou => "Needs you",
            Self::Takeover => "You are in control",
            Self::HandBack => "Back to the Verifier",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    Passed,
    Failed,
    Skipped,
    Unmarked,
    InProgress,
}
impl Tick {
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Unmarked => "unmarked",
            Self::InProgress => "in-progress",
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterButton {
    TakeControl,
    Done,
    OpenVideo,
    OpenFolder,
    Dismiss,
}
impl FooterButton {
    pub fn text(self) -> &'static str {
        match self {
            Self::TakeControl => "Take control",
            Self::Done => "Done",
            Self::OpenVideo => "Open video",
            Self::OpenFolder => "Open folder",
            Self::Dismiss => "Dismiss",
        }
    }
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
    pub button: Option<FooterButton>,
    pub keycaps: bool,
    pub buttons: Vec<FooterButton>,
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
    let seconds = match &snapshot.state {
        SessionState::Rendering { elapsed, .. } => Some(elapsed.0 / 1000),
        SessionState::Ended(ended) => Some(ended.elapsed.0 / 1000),
        SessionState::Opening | SessionState::Open | SessionState::Closing => frozen,
    }
    .unwrap_or_else(|| now.signed_duration_since(info.opened).num_seconds().max(0) as u64);
    let (mut label, mut detail, elapsed_label) = match &snapshot.state {
        SessionState::Rendering { percent, .. } => {
            (StateLabel::Rendering, format!("{}%", percent.0), "Ran")
        }
        SessionState::Ended(ended) => {
            use crate::checklist::{SessionEnd, Verdict};
            let label = match ended.end {
                SessionEnd::Verdict(Verdict::Pass) => StateLabel::Pass,
                SessionEnd::Verdict(Verdict::NeedsFixes) => StateLabel::NeedsFixes,
                SessionEnd::Verdict(Verdict::PartlyVerified) => StateLabel::PartlyVerified,
                SessionEnd::Incomplete(_) => StateLabel::Incomplete,
            };
            let detail = match &ended.end {
                SessionEnd::Verdict(Verdict::Pass) => match &ended.verification {
                    Some(_) => format!(
                        "{} video · {} action{}",
                        duration(ended.video_length.0 / 1000),
                        ended.actions,
                        if ended.actions == 1 { "" } else { "s" }
                    ),
                    None => format!(
                        "{} action{}",
                        ended.actions,
                        if ended.actions == 1 { "" } else { "s" }
                    ),
                },
                SessionEnd::Incomplete(reason) => reason.short(),
                SessionEnd::Verdict(Verdict::NeedsFixes | Verdict::PartlyVerified) => {
                    let mut failed = 0;
                    let mut skipped = 0;
                    for c in &snapshot.checks {
                        match &c.mark {
                            Mark::Unmarked => {}
                            Mark::Outcome { outcome, .. } => match outcome {
                                CheckOutcome::Passed => {}
                                CheckOutcome::Failed => failed += 1,
                                CheckOutcome::Skipped => skipped += 1,
                            },
                        }
                    }
                    let mut parts = Vec::new();
                    if failed > 0 {
                        parts.push(format!(
                            "{failed} check{} failed",
                            if failed == 1 { "" } else { "s" }
                        ));
                    }
                    if skipped > 0 {
                        parts.push(if failed > 0 {
                            format!("{skipped} skipped")
                        } else {
                            format!(
                                "{skipped} check{} skipped",
                                if skipped == 1 { "" } else { "s" }
                            )
                        });
                    }
                    let mut detail = parts.join(", ");
                    if matches!(ended.end, SessionEnd::Verdict(Verdict::NeedsFixes))
                        && let Some(attempt) = ended.attempt
                    {
                        if !detail.is_empty() {
                            detail.push_str(" · ");
                        }
                        detail.push_str(&format!("attempt {}", attempt.0));
                        if attempt.0 <= 3 {
                            detail.push_str(" of 3");
                        }
                    }
                    detail
                }
            };
            (label, detail, "Ran")
        }
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
        SessionState::Open if snapshot.reminded => (
            StateLabel::Reminded,
            format!("{THRESHOLD} actions since the last mark"),
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
    let mut button = (snapshot.state == SessionState::Open).then_some(FooterButton::TakeControl);
    let mut keycaps = false;
    if snapshot.state == SessionState::Open {
        match &snapshot.control {
            crate::ipc::ControlState::Verifier => {}
            crate::ipc::ControlState::HelpRequest { reason, start_ms } => {
                label = StateLabel::NeedsYou;
                detail = format!(
                    "{reason} · waiting {}",
                    duration(seconds.saturating_sub(start_ms.0 / 1000))
                );
            }
            crate::ipc::ControlState::Takeover {
                start_ms,
                shortcuts,
            } => {
                label = StateLabel::Takeover;
                let shortcuts = match shortcuts {
                    crate::ipc::HostShortcuts::Captured => {
                        keycaps = true;
                        "host shortcuts go to the session"
                    }
                    crate::ipc::HostShortcuts::Live => "host shortcuts stay live (no keek submap)",
                };
                detail = format!(
                    "{} · {shortcuts}",
                    duration(seconds.saturating_sub(start_ms.0 / 1000))
                );
                button = Some(FooterButton::Done);
            }
            crate::ipc::ControlState::HandBack {
                end_ms,
                length,
                result,
            } => {
                if hover == Hover::Over {
                    label = StateLabel::ViewOnly;
                    detail = "Use Take control to interact.".into();
                } else {
                    label = StateLabel::HandBack;
                    use crate::ipc::HandBackResult;
                    let held = duration(length.0 / 1000);
                    detail = match result {
                        HandBackResult::Handed => format!("Handed back after {held}."),
                        HandBackResult::HelpAnswered => {
                            format!("Handed back after {held}. Help Request answered.")
                        }
                        HandBackResult::Unchanged | HandBackResult::HelpUnanswered => {
                            "The screen looks unchanged. The Verifier was told.".into()
                        }
                    };
                    if let Some(action) = &snapshot.last_action
                        && action.time_ms > *end_ms
                    {
                        detail.push_str(&format!(
                            " · {} · {} ago",
                            action_words(&action.tool),
                            duration(seconds.saturating_sub(action.time_ms.0 / 1000))
                        ));
                    }
                }
            }
        }
    }
    let show_in_progress = snapshot.state == SessionState::Open
        && matches!(
            snapshot.control,
            crate::ipc::ControlState::Verifier | crate::ipc::ControlState::HandBack { .. }
        );
    let mut counts = [0; 4];
    let mut ticks = Vec::new();
    for check in &snapshot.checks {
        let (index, tick) = match &check.mark {
            Mark::Unmarked if show_in_progress && counts[3] == 0 => (3, Tick::InProgress),
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
        button,
        keycaps,
        buttons: match &snapshot.state {
            SessionState::Ended(ended) => {
                let mut buttons = Vec::new();
                if ended.verification.is_some() {
                    buttons.push(FooterButton::OpenVideo);
                }
                if ended.folder.is_some() {
                    buttons.push(FooterButton::OpenFolder);
                }
                buttons.push(FooterButton::Dismiss);
                buttons
            }
            SessionState::Opening
            | SessionState::Open
            | SessionState::Closing
            | SessionState::Rendering { .. } => button.into_iter().collect(),
        },
    }
}
#[cfg(test)]
#[path = "footer_view_tests.rs"]
mod test_cases;
