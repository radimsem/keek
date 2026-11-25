use super::SessionInfo;
use super::footer_view::{CheckedView, Hover, StateLabel, Tick, check_ticks, for_session};
use crate::ipc::{ControlState, SessionSnapshot, SessionState};
use crate::slots::{RailShape, Slot};
use chrono::{DateTime, Utc};

pub const LOCKED_NOTE: &str = "Other sessions unlock when you press Done.";
pub(super) const HOVER_WIDTH: i32 = 440;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverSpot {
    LeftOf { x: i32, top: i32 },
    Above { x: i32, bottom: i32 },
}

pub fn hover_spot(card: Slot, shape: RailShape) -> HoverSpot {
    match shape {
        RailShape::Column => HoverSpot::LeftOf {
            x: card.x - HOVER_WIDTH - 12,
            top: card.y,
        },
        RailShape::Row => HoverSpot::Above {
            x: card.x,
            bottom: card.y - 12,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteSpot {
    pub x: i32,
    pub top: i32,
    pub width: i32,
}

pub fn locked_note_spot(rail: &[Slot], shape: RailShape) -> Option<NoteSpot> {
    let first = *rail.first()?;
    match shape {
        RailShape::Column => {
            let bottom = rail.iter().map(|card| card.y + card.height).max()?;
            Some(NoteSpot {
                x: first.x,
                top: bottom + 16,
                width: first.width,
            })
        }
        RailShape::Row => {
            let left = rail.iter().map(|card| card.x).min()?;
            let right = rail.iter().map(|card| card.x + card.width).max()?;
            Some(NoteSpot {
                x: left + 8,
                top: first.y + 8,
                width: right - left - 16,
            })
        }
    }
}

pub struct RailCardView {
    pub project: String,
    pub branch: String,
    pub elapsed: String,
    pub needs_you: Option<String>,
    pub line: String,
    pub verdict: Option<StateLabel>,
    pub checked: Option<CheckedView>,
    pub keys: Option<[String; 2]>,
    pub still: Option<String>,
    pub accessible: String,
    pub locked: bool,
    pub hover: RailHover,
}

#[derive(PartialEq, Eq)]
pub struct RailHover {
    pub goal: String,
    pub checks: Vec<HoverCheck>,
    pub hint: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverCheck {
    pub text: String,
    pub mark: HoverMark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverMark {
    Passed,
    Failed,
    Skipped,
    Unmarked,
    Checking,
}

const SHORTCUTS: usize = 4;

/// The Super+F1 to Super+F4 binds the developer has added; keek ships none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShowKeys([bool; SHORTCUTS]);

impl ShowKeys {
    pub fn from_binds(binds: &serde_json::Value) -> Self {
        Self(std::array::from_fn(|index| {
            crate::hypr::has_super_bind(binds, "", &function_key(index + 1))
        }))
    }

    fn keys(self, slot: super::SlotId) -> Option<[String; 2]> {
        self.0
            .get(slot.0)
            .copied()?
            .then(|| ["Super".into(), function_key(slot.0 + 1)])
    }
}

fn function_key(number: usize) -> String {
    format!("F{number}")
}

pub fn rail_locked<'a>(snapshots: impl IntoIterator<Item = &'a SessionSnapshot>) -> bool {
    snapshots.into_iter().any(|snapshot| {
        snapshot.state == SessionState::Open
            && matches!(snapshot.control, ControlState::Takeover { .. })
    })
}

pub fn for_rail(
    info: &SessionInfo,
    snapshot: &SessionSnapshot,
    now: DateTime<Utc>,
    still_taken: Option<DateTime<Utc>>,
    locked: bool,
    show_keys: ShowKeys,
) -> RailCardView {
    let footer = for_session(info, snapshot, now, Hover::Away, None);
    let verdict = matches!(snapshot.state, SessionState::Ended(_)).then_some(footer.label);
    let needs_you = match &snapshot.control {
        ControlState::HelpRequest { reason, .. } if footer.label == StateLabel::NeedsYou => {
            Some(reason.clone())
        }
        ControlState::HelpRequest { .. }
        | ControlState::Verifier
        | ControlState::Takeover { .. }
        | ControlState::HandBack { .. } => None,
    };
    let checks = snapshot
        .checks
        .iter()
        .zip(check_ticks(snapshot))
        .map(|(check, tick)| HoverCheck {
            text: check.text.clone(),
            mark: match tick {
                Tick::Passed => HoverMark::Passed,
                Tick::Failed => HoverMark::Failed,
                Tick::Skipped => HoverMark::Skipped,
                Tick::Unmarked => HoverMark::Unmarked,
                Tick::InProgress => HoverMark::Checking,
            },
        })
        .collect();
    let still = still_taken.filter(|_| verdict.is_none()).map(|taken| {
        let seconds = (now - taken).num_seconds().max(0);
        if seconds < 60 {
            format!("Still · {seconds} s ago")
        } else {
            format!("Still · {} min ago", seconds / 60)
        }
    });
    RailCardView {
        accessible: format!("Show {}", footer.project),
        project: footer.project,
        branch: footer.branch,
        elapsed: footer.elapsed,
        needs_you,
        line: verdict.map_or(footer.detail, |label| label.text().into()),
        verdict,
        checked: footer.checked,
        keys: show_keys.keys(info.slot),
        still,
        locked,
        hover: RailHover {
            goal: footer.goal,
            checks,
            hint: "Click to show this session.",
        },
    }
}

#[cfg(test)]
#[path = "rail_view_tests.rs"]
mod tests;
