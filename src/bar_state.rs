use crate::{
    checklist::{Mark, SessionEnd, Verdict},
    ipc::{ControlState, SessionSnapshot, SessionState, TakeoverPhase},
    timeline::CheckOutcome,
    viewer::{
        SessionInfo,
        footer_view::SessionColour,
        rail_view::{ShowKeys, for_rail},
    },
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub const BAR_STATE_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum BarStateError {
    #[error(
        "the bar state could not be written to {}: {source}. Check that $XDG_RUNTIME_DIR/keek is writable.",
        path.display()
    )]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BarState {
    pub version: u32,
    pub sessions: Vec<BarSession>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BarSession {
    pub n: usize,
    pub project: String,
    pub branch: Option<String>,
    pub color: &'static str,
    pub opened_at: DateTime<Utc>,
    pub state: BarSessionState,
    pub verdict: Option<BarVerdict>,
    pub detail: String,
    pub checks: String,
    pub in_focus: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BarSessionState {
    Working,
    NeedsYou,
    Takeover,
    TakeoverPaused,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BarVerdict {
    Pass,
    NeedsFixes,
    PartlyVerified,
    Incomplete,
}

pub fn bar_session(
    info: &SessionInfo,
    snapshot: &SessionSnapshot,
    in_focus: bool,
    now: DateTime<Utc>,
) -> BarSession {
    let (state, verdict) = match (&snapshot.state, &snapshot.control) {
        (SessionState::Ended(ended), _) => (BarSessionState::Ended, Some(verdict(&ended.end))),
        (
            SessionState::Opening
            | SessionState::Open
            | SessionState::Closing
            | SessionState::Rendering { .. },
            ControlState::HelpRequest { .. },
        ) => (BarSessionState::NeedsYou, None),
        (
            SessionState::Opening
            | SessionState::Open
            | SessionState::Closing
            | SessionState::Rendering { .. },
            ControlState::Takeover {
                phase: TakeoverPhase::Paused { .. },
                ..
            },
        ) => (BarSessionState::TakeoverPaused, None),
        (
            SessionState::Opening
            | SessionState::Open
            | SessionState::Closing
            | SessionState::Rendering { .. },
            ControlState::Takeover {
                phase: TakeoverPhase::Running,
                ..
            },
        ) => (BarSessionState::Takeover, None),
        (
            SessionState::Opening
            | SessionState::Open
            | SessionState::Closing
            | SessionState::Rendering { .. },
            ControlState::Verifier | ControlState::HandBack { .. },
        ) => (BarSessionState::Working, None),
    };
    let detail = match (&snapshot.control, state) {
        (ControlState::HelpRequest { reason, .. }, BarSessionState::NeedsYou) => reason.clone(),
        _ => for_rail(info, snapshot, now, None, false, ShowKeys::default()).line,
    };
    BarSession {
        n: info.slot.0 + 1,
        project: info.project.clone(),
        branch: info.branch.clone(),
        color: SessionColour::for_slot(info.slot).color(),
        opened_at: info.opened,
        state,
        verdict,
        detail,
        checks: snapshot
            .checks
            .iter()
            .map(|check| letter(&check.mark))
            .collect(),
        in_focus,
    }
}

fn verdict(end: &SessionEnd) -> BarVerdict {
    match end {
        SessionEnd::Verdict(Verdict::Pass) => BarVerdict::Pass,
        SessionEnd::Verdict(Verdict::NeedsFixes) => BarVerdict::NeedsFixes,
        SessionEnd::Verdict(Verdict::PartlyVerified) => BarVerdict::PartlyVerified,
        SessionEnd::Incomplete(_) => BarVerdict::Incomplete,
    }
}

fn letter(mark: &Mark) -> char {
    match mark {
        Mark::Unmarked => '-',
        Mark::Outcome { outcome, .. } => match outcome {
            CheckOutcome::Passed => 'p',
            CheckOutcome::Failed => 'f',
            CheckOutcome::Skipped => 's',
        },
    }
}

pub fn write(path: &Path, state: &BarState) -> Result<(), BarStateError> {
    let fail = |source| BarStateError::Write {
        path: path.to_owned(),
        source,
    };
    let bytes = serde_json::to_vec(state).map_err(|e| fail(e.into()))?;
    let temporary = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(fail)?;
    file.write_all(&bytes).map_err(fail)?;
    std::fs::rename(&temporary, path).map_err(fail)
}

pub fn remove(path: &Path) -> Result<(), BarStateError> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(BarStateError::Write {
            path: path.to_owned(),
            source: error,
        }),
        Ok(()) | Err(_) => Ok(()),
    }
}

#[cfg(test)]
#[path = "bar_state_tests.rs"]
mod tests;
