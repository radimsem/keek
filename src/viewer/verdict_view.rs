use super::footer_view::Tick;
use crate::{
    checklist::{Mark, SessionEnd, Verdict},
    ipc::{Percent, SessionSnapshot, SessionState},
    timeline::CheckOutcome,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Pass,
    NeedsFixes,
    PartlyVerified,
    Dim,
}
impl Tone {
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Pass => "passed",
            Self::NeedsFixes => "failed",
            Self::PartlyVerified => "skipped",
            Self::Dim => "dim",
        }
    }
}
pub type Icon = Tone;
pub struct CheckRow {
    pub text: String,
    pub added: bool,
    pub comment: Option<String>,
    pub tick: Tick,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelMode {
    Rendering(Percent),
    Ended,
}
pub struct PanelView {
    pub title: String,
    pub subtitle: String,
    pub tally: String,
    pub tone: Tone,
    pub icon: Icon,
    pub mode: PanelMode,
    pub rows: Vec<CheckRow>,
}
pub fn for_snapshot(snapshot: &SessionSnapshot) -> Option<PanelView> {
    let ended = match &snapshot.state {
        SessionState::Opening | SessionState::Open | SessionState::Closing => return None,
        SessionState::Rendering { percent, .. } => {
            return Some(PanelView {
                title: "Rendering the verification video".into(),
                subtitle: String::new(),
                tally: format!("{}%", percent.0),
                tone: Tone::Dim,
                icon: Icon::Dim,
                mode: PanelMode::Rendering(*percent),
                rows: vec![],
            });
        }
        SessionState::Ended(ended) => ended,
    };
    let mut counts = [0usize; 4];
    let rows = snapshot
        .checks
        .iter()
        .map(|c| {
            let (i, tick, comment) = match &c.mark {
                Mark::Unmarked => (3, Tick::Unmarked, None),
                Mark::Outcome {
                    outcome, comment, ..
                } => match outcome {
                    CheckOutcome::Passed => (0, Tick::Passed, None),
                    CheckOutcome::Failed => (1, Tick::Failed, Some(comment.clone())),
                    CheckOutcome::Skipped => (2, Tick::Skipped, Some(comment.clone())),
                },
            };
            counts[i] += 1;
            CheckRow {
                text: c.text.clone(),
                added: c.added,
                comment,
                tick,
            }
        })
        .collect();
    let tone = match ended.end {
        SessionEnd::Verdict(Verdict::Pass) => Tone::Pass,
        SessionEnd::Verdict(Verdict::NeedsFixes) => Tone::NeedsFixes,
        SessionEnd::Verdict(Verdict::PartlyVerified) => Tone::PartlyVerified,
        SessionEnd::Incomplete(_) => Tone::Dim,
    };
    let subtitle = match &ended.end {
        SessionEnd::Incomplete(reason) => format!("{}, session ended", reason.short()),
        SessionEnd::Verdict(verdict) => format!(
            "{} {} at {}{}",
            if *verdict == Verdict::Pass {
                "Verified"
            } else {
                "Checked"
            },
            ended.revision.branch.as_deref().unwrap_or("detached HEAD"),
            ended
                .revision
                .commit
                .get(..7)
                .unwrap_or(&ended.revision.commit),
            if ended.revision.dirty {
                " + uncommitted changes"
            } else {
                ""
            }
        ),
    };
    let tally = match ended.end {
        SessionEnd::Incomplete(_) => format!(
            "{} of {} checked",
            counts[0] + counts[1] + counts[2],
            snapshot.checks.len()
        ),
        SessionEnd::Verdict(verdict) => {
            let mut parts = Vec::new();
            if verdict != Verdict::NeedsFixes && counts[0] > 0 {
                parts.push(format!("{} passed", counts[0]));
            }
            if counts[1] > 0 {
                parts.push(format!("{} failed", counts[1]));
            }
            if counts[2] > 0 {
                parts.push(format!("{} skipped", counts[2]));
            }
            let mut text = parts.join(", ");
            if verdict == Verdict::NeedsFixes
                && let Some(attempt) = ended.attempt
            {
                text.push_str(&format!(" · attempt {}", attempt.0));
                if attempt.0 <= 3 {
                    text.push_str(" of 3");
                }
            }
            text
        }
    };
    Some(PanelView {
        title: ended.end.label(),
        subtitle,
        tally,
        tone,
        icon: tone,
        mode: PanelMode::Ended,
        rows,
    })
}
#[cfg(test)]
#[path = "verdict_view_tests.rs"]
pub(super) mod tests;
