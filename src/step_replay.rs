//! Intents and scoring for the offline replay in `examples/delegate_replay.rs`.

use crate::decision_model::Decision;
use crate::step_candidates::Candidate;
use crate::step_policy::{StepDecision, decide_action};

/// What a replayed Delegated Step should end in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// The candidate id of the control the filed click used.
    Control(String),
    /// No offered control is meant, so the right outcome is handing the step back.
    HandBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    RightAct,
    WrongAct,
    HandedBack,
}

/// "click the {word} {role}", where `word` is the label's longest word, lowercased.
///
/// It drops the rest of the label, so controls that share the word compete. `None` when the
/// label has no letter or digit.
#[must_use]
pub fn partial_intent(role: &str, label: &str) -> Option<String> {
    let lower = label.to_lowercase();
    let word = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .fold(None::<&str>, |best, word| match best {
            Some(best) if best.chars().count() >= word.chars().count() => Some(best),
            _ => Some(word),
        })?;
    Some(format!("click the {word} {role}"))
}

/// The first of `others` that appears nowhere in `tree`, in any case.
///
/// Matching anywhere, not only whole labels, leaves out near matches such as "Save" on a screen
/// with "Save changes", so an intent built from the result names no control on that screen.
#[must_use]
pub fn absent_label<'a>(tree: &str, others: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let tree = tree.to_lowercase();
    others
        .into_iter()
        .find(|label| !label.trim().is_empty() && !tree.contains(&label.to_lowercase()))
}

/// Applies keek's acting rule at `threshold` and says whether the result was what `expected` asks.
#[must_use]
pub fn outcome(
    decision: &Decision,
    candidates: &[Candidate],
    expected: &Expected,
    threshold: f64,
) -> Outcome {
    match decide_action(decision, candidates, threshold) {
        StepDecision::HandBack(_) => Outcome::HandedBack,
        StepDecision::Act { candidate, .. } => match expected {
            Expected::Control(id) if *id == candidate.id => Outcome::RightAct,
            Expected::Control(_) | Expected::HandBack => Outcome::WrongAct,
        },
    }
}

#[cfg(test)]
#[path = "step_replay_tests.rs"]
mod tests;
