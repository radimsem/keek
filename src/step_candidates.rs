use std::cmp::Reverse;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CANDIDATE_CAP: usize = 24;

// AT-SPI role strings as Cua Driver emits them on Linux.
const CLICK_ROLES: &[&str] = &[
    "button",
    "toggle button",
    "link",
    "check box",
    "radio button",
    "menu item",
    "page tab",
    "list item",
    "entry",
    "password text",
    "combo box",
];
const TYPE_ROLES: &[&str] = &["entry", "password text", "text"];
const SELECT_ROLES: &[&str] = &["combo box", "list"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepAction {
    Click,
    DoubleClick,
    RightClick,
    Type,
    Select,
}

impl StepAction {
    fn roles(self) -> &'static [&'static str] {
        match self {
            Self::Click | Self::DoubleClick | Self::RightClick => CLICK_ROLES,
            Self::Type => TYPE_ROLES,
            Self::Select => SELECT_ROLES,
        }
    }
}

/// An element's frame in window coordinates, as Cua reports it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub id: String,
    /// Never sent to the Decision Model.
    pub element_token: String,
    pub role: String,
    pub label: Option<String>,
    pub value: Option<String>,
    pub frame: Frame,
    pub in_web_content: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CandidatesError {
    #[error(
        "get_window_state returned no element list for this window. Call get_window_state and act on the window yourself."
    )]
    NoElements,
}

#[derive(Deserialize)]
struct Element {
    role: String,
    element_token: Option<String>,
    label: Option<String>,
    value: Option<String>,
    enabled: Option<bool>,
    in_web_content: Option<bool>,
    frame: Option<Frame>,
}

/// Builds at most `CANDIDATE_CAP` candidates the action can apply to, best intent match first.
///
/// `state` is a `get_window_state` result, with or without its `structuredContent` envelope.
/// Candidates carry values as Cua reports them, so the caller masks secrets before sending any.
pub fn candidates(
    state: &Value,
    action: StepAction,
    intent: &str,
) -> Result<Vec<Candidate>, CandidatesError> {
    let window = state.get("structuredContent").unwrap_or(state);
    let elements = window
        .get("elements")
        .and_then(Value::as_array)
        .ok_or(CandidatesError::NoElements)?;
    let intent_words = words(intent);

    let mut ranked: Vec<(usize, Candidate)> = elements
        .iter()
        // An element that doesn't parse is left out, so one odd element doesn't lose the window.
        .filter_map(|raw| Element::deserialize(raw).ok())
        .filter_map(|element| into_candidate(element, action))
        .map(|candidate| (score(&candidate, &intent_words), candidate))
        .collect();
    ranked.sort_by_key(|(score, _)| Reverse(*score));
    ranked.truncate(CANDIDATE_CAP);

    Ok(ranked
        .into_iter()
        .enumerate()
        .map(|(index, (_, mut candidate))| {
            candidate.id = format!("c{}", index + 1);
            candidate
        })
        .collect())
}

fn into_candidate(element: Element, action: StepAction) -> Option<Candidate> {
    if element.enabled == Some(false) || !action.roles().contains(&element.role.as_str()) {
        return None;
    }
    Some(Candidate {
        id: String::new(),
        element_token: element.element_token?,
        role: element.role,
        label: element.label,
        value: element.value,
        frame: element.frame?,
        in_web_content: element.in_web_content.unwrap_or(false),
    })
}

fn score(candidate: &Candidate, intent_words: &HashSet<String>) -> usize {
    let text = [candidate.label.as_deref(), candidate.value.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let found = words(&text);
    intent_words.intersection(&found).count()
}

fn words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
#[path = "step_candidates_tests.rs"]
mod tests;
