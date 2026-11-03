use crate::timeline::{CheckId, CheckOutcome, Millis};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub id: CheckId,
    pub text: String,
    pub added: bool,
    pub mark: Mark,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Unmarked,
    Outcome {
        outcome: CheckOutcome,
        comment: String,
        at: Millis,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Pass,
    NeedsFixes,
    PartlyVerified,
}
impl Verdict {
    pub fn filename_word(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::NeedsFixes => "needs-fixes",
            Self::PartlyVerified => "partly-verified",
        }
    }
}
impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pass => "Pass",
            Self::NeedsFixes => "Needs fixes",
            Self::PartlyVerified => "Partly verified",
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IncompleteReason {
    Idle { limit: std::time::Duration },
    OpenerLeft,
    DesktopStopped,
    DaemonStopped,
}
fn idle_duration(limit: std::time::Duration) -> (String, &'static str, &'static str) {
    let seconds = limit.as_secs();
    if seconds > 0 && seconds.is_multiple_of(60) && limit.subsec_nanos() == 0 {
        let minutes = seconds / 60;
        (
            minutes.to_string(),
            if minutes == 1 { "minute" } else { "minutes" },
            "min",
        )
    } else {
        (
            limit.as_secs_f64().to_string(),
            if limit == std::time::Duration::from_secs(1) {
                "second"
            } else {
                "seconds"
            },
            "s",
        )
    }
}
impl IncompleteReason {
    pub fn refusal(&self) -> String {
        let cause = match self {
            Self::Idle { .. } => "The session was idle",
            Self::OpenerLeft => "The coding agent disconnected",
            Self::DesktopStopped => "The session's desktop stopped",
            Self::DaemonStopped => "The keek daemon stopped",
        };
        format!(
            "{cause}, so keek ended the session and filed it as an Incomplete Session. Open a new Agent Session."
        )
    }
    pub fn sentence(&self) -> String {
        match self {
            Self::Idle { limit } => {
                let (amount, unit, _) = idle_duration(*limit);
                format!("keek ended the session after {amount} {unit} without a tool call.")
            }
            Self::OpenerLeft => {
                "keek ended the session because the coding agent's keek mcp disconnected.".into()
            }
            Self::DaemonStopped => {
                "The keek daemon stopped during the session, and the next daemon filed it.".into()
            }
            Self::DesktopStopped => "The session's desktop stopped before close_session.".into(),
        }
    }
    pub fn short(&self) -> String {
        match self {
            Self::Idle { limit } => {
                let (amount, _, unit) = idle_duration(*limit);
                format!("No tool call for {amount} {unit}")
            }
            Self::OpenerLeft => "Coding agent disconnected".into(),
            Self::DesktopStopped => "Desktop stopped".into(),
            Self::DaemonStopped => "Daemon stopped".into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEnd {
    Verdict(Verdict),
    Incomplete(IncompleteReason),
}
impl SessionEnd {
    pub fn filename_word(&self) -> &'static str {
        match self {
            Self::Verdict(v) => v.filename_word(),
            Self::Incomplete(_) => "incomplete",
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Verdict(v) => v.to_string(),
            Self::Incomplete(_) => "Incomplete".into(),
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ChecklistError {
    #[error("Cannot add an empty Check. Call add_check with observable acceptance text.")]
    EmptyText,
    #[error("Cannot mark unknown Check {id}. Known ids: {known}. Call mark_check with a known id.")]
    Unknown { id: u32, known: String },
    #[error(
        "Cannot mark this Check: failed and skipped need a comment, and every comment must be one line. Call mark_check with a one-line comment."
    )]
    Comment,
    #[error(
        "Cannot close the Agent Session: unmarked Checks: {0}. Call mark_check for each one, then close_session."
    )]
    Unmarked(String),
    #[error(
        "Cannot close the Agent Session with no Checks. Call add_check, then mark_check before close_session."
    )]
    Empty,
}
pub struct Checklist {
    checks: Vec<Check>,
}
impl Checklist {
    pub fn new(texts: Vec<String>) -> Self {
        Self {
            checks: texts
                .into_iter()
                .enumerate()
                .map(|(i, text)| Check {
                    id: CheckId(i as u32 + 1),
                    text,
                    added: false,
                    mark: Mark::Unmarked,
                })
                .collect(),
        }
    }
    pub fn add(&mut self, text: String) -> Result<CheckId, ChecklistError> {
        if text.trim().is_empty() {
            return Err(ChecklistError::EmptyText);
        }
        let id = CheckId(self.checks.len() as u32 + 1);
        self.checks.push(Check {
            id,
            text,
            added: true,
            mark: Mark::Unmarked,
        });
        Ok(id)
    }
    pub fn mark(
        &mut self,
        id: CheckId,
        outcome: CheckOutcome,
        comment: String,
        at: Millis,
    ) -> Result<(), ChecklistError> {
        let known = self
            .checks
            .iter()
            .map(|c| c.id.0.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let check = self
            .checks
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(ChecklistError::Unknown { id: id.0, known })?;
        let required = match outcome {
            CheckOutcome::Passed => false,
            CheckOutcome::Failed | CheckOutcome::Skipped => true,
        };
        if comment.contains(['\n', '\r']) || (required && comment.trim().is_empty()) {
            return Err(ChecklistError::Comment);
        }
        check.mark = Mark::Outcome {
            outcome,
            comment,
            at,
        };
        Ok(())
    }
    pub fn checks(&self) -> &[Check] {
        &self.checks
    }
    pub fn verdict(&self) -> Result<Verdict, ChecklistError> {
        if self.checks.is_empty() {
            return Err(ChecklistError::Empty);
        }
        let unmarked = self
            .checks
            .iter()
            .filter(|c| matches!(c.mark, Mark::Unmarked))
            .map(|c| format!("{}: {}", c.id.0, c.text))
            .collect::<Vec<_>>();
        if !unmarked.is_empty() {
            return Err(ChecklistError::Unmarked(unmarked.join(", ")));
        }
        let mut verdict = Verdict::Pass;
        for check in &self.checks {
            match &check.mark {
                Mark::Outcome {
                    outcome: CheckOutcome::Failed,
                    ..
                } => return Ok(Verdict::NeedsFixes),
                Mark::Outcome {
                    outcome: CheckOutcome::Skipped,
                    ..
                } => verdict = Verdict::PartlyVerified,
                Mark::Outcome {
                    outcome: CheckOutcome::Passed,
                    ..
                }
                | Mark::Unmarked => {}
            }
        }
        Ok(verdict)
    }
}

#[cfg(test)]
#[path = "checklist_tests.rs"]
mod tests;
