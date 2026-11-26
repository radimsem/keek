use serde::{Deserialize, Serialize};

use crate::decision_model::{ABSTAIN, Decision, REOBSERVE};
use crate::step_candidates::Candidate;

#[derive(Debug, Clone, PartialEq)]
pub enum StepDecision {
    Act {
        candidate: Candidate,
        probability: f64,
    },
    HandBack(HandBackReason),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandBackReason {
    Abstained,
    Reobserve,
    LowConfidence { probability: f64 },
    NoMatch { matches: f64 },
    NotOffered { choice: String },
    ModelFailed(String),
}

/// Acts only when the pick is an offered candidate and both its probability and
/// the Noul probability reach `threshold`. When several reasons apply, the
/// reported one is, in order: abstained or reobserve, not offered, low
/// confidence, no match. A missing, NaN or out-of-range probability never acts,
/// whatever the threshold, and is reported as 0.
pub fn decide_action(
    decision: &Decision,
    candidates: &[Candidate],
    threshold: f64,
) -> StepDecision {
    let choice = decision.choice.as_str();
    if choice == ABSTAIN {
        return StepDecision::HandBack(HandBackReason::Abstained);
    }
    if choice == REOBSERVE {
        return StepDecision::HandBack(HandBackReason::Reobserve);
    }
    let Some(candidate) = candidates.iter().find(|c| c.id == choice) else {
        return StepDecision::HandBack(HandBackReason::NotOffered {
            choice: decision.choice.clone(),
        });
    };
    let probability = valid(decision.probabilities.get(choice).copied());
    let Some(probability) = probability.filter(|p| *p >= threshold) else {
        return StepDecision::HandBack(HandBackReason::LowConfidence {
            probability: probability.unwrap_or(0.0),
        });
    };
    let matches = valid(Some(decision.matches));
    if !matches.is_some_and(|m| m >= threshold) {
        return StepDecision::HandBack(HandBackReason::NoMatch {
            matches: matches.unwrap_or(0.0),
        });
    }
    StepDecision::Act {
        candidate: candidate.clone(),
        probability,
    }
}

fn valid(probability: Option<f64>) -> Option<f64> {
    probability.filter(|p| (0.0..=1.0).contains(p))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::*;
    use crate::step_candidates::Frame;

    fn candidate(id: &str) -> Candidate {
        Candidate {
            id: id.to_string(),
            element_token: format!("tok-{id}"),
            role: "button".to_string(),
            label: Some("Save".to_string()),
            value: None,
            frame: Frame {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            in_web_content: false,
        }
    }

    fn candidates() -> Vec<Candidate> {
        vec![candidate("c1"), candidate("c2")]
    }

    fn decision(choice: &str, probabilities: &[(&str, f64)], matches: f64) -> Decision {
        Decision {
            choice: choice.to_string(),
            probabilities: probabilities
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<BTreeMap<_, _>>(),
            matches,
            latency: Duration::from_millis(5),
        }
    }

    fn hand_back(decision: &Decision, threshold: f64) -> HandBackReason {
        match decide_action(decision, &candidates(), threshold) {
            StepDecision::HandBack(reason) => reason,
            StepDecision::Act { .. } => unreachable!("expected a hand back"),
        }
    }

    #[test]
    fn a_confident_offered_pick_with_a_confident_match_acts() {
        let d = decision("c2", &[("c1", 0.05), ("c2", 0.95)], 0.9);
        assert_eq!(
            decide_action(&d, &candidates(), 0.8),
            StepDecision::Act {
                candidate: candidate("c2"),
                probability: 0.95,
            }
        );
    }

    #[test]
    fn a_pick_below_the_threshold_hands_back() {
        let d = decision("c1", &[("c1", 0.6), ("c2", 0.4)], 0.9);
        assert_eq!(
            hand_back(&d, 0.8),
            HandBackReason::LowConfidence { probability: 0.6 }
        );
    }

    #[test]
    fn a_low_noul_hands_back_even_with_a_confident_pick() {
        let d = decision("c1", &[("c1", 0.99)], 0.2);
        assert_eq!(hand_back(&d, 0.8), HandBackReason::NoMatch { matches: 0.2 });
    }

    #[test]
    fn abstain_hands_back_as_abstained() {
        let d = decision("abstain", &[("abstain", 0.99)], 0.99);
        assert_eq!(hand_back(&d, 0.8), HandBackReason::Abstained);
    }

    #[test]
    fn reobserve_hands_back_as_reobserve() {
        let d = decision("reobserve", &[("reobserve", 0.99)], 0.99);
        assert_eq!(hand_back(&d, 0.8), HandBackReason::Reobserve);
    }

    #[test]
    fn a_pick_that_was_not_offered_hands_back_and_never_acts() {
        let d = decision("c99", &[("c99", 0.99)], 0.99);
        assert_eq!(
            hand_back(&d, 0.8),
            HandBackReason::NotOffered {
                choice: "c99".to_string()
            }
        );
    }

    #[test]
    fn a_pick_with_no_probability_never_acts() {
        let d = decision("c1", &[("c2", 0.99)], 0.99);
        assert_eq!(
            hand_back(&d, 0.8),
            HandBackReason::LowConfidence { probability: 0.0 }
        );
    }

    #[test]
    fn a_nan_or_out_of_range_probability_never_acts() {
        let nan_pick = decision("c1", &[("c1", f64::NAN)], 0.99);
        assert_eq!(
            hand_back(&nan_pick, 0.0),
            HandBackReason::LowConfidence { probability: 0.0 }
        );
        let high_pick = decision("c1", &[("c1", 1.5)], 0.99);
        assert_eq!(
            hand_back(&high_pick, 0.8),
            HandBackReason::LowConfidence { probability: 0.0 }
        );
        let nan_noul = decision("c1", &[("c1", 0.99)], f64::NAN);
        assert_eq!(
            hand_back(&nan_noul, 0.0),
            HandBackReason::NoMatch { matches: 0.0 }
        );
    }

    #[test]
    fn the_reason_follows_the_documented_order() {
        let not_offered = decision("c99", &[("c99", 0.1)], 0.1);
        assert_eq!(
            hand_back(&not_offered, 0.8),
            HandBackReason::NotOffered {
                choice: "c99".to_string()
            }
        );
        let low_both = decision("c1", &[("c1", 0.1)], 0.1);
        assert_eq!(
            hand_back(&low_both, 0.8),
            HandBackReason::LowConfidence { probability: 0.1 }
        );
    }

    #[test]
    fn a_hand_back_reason_round_trips_through_json() {
        let reason = HandBackReason::NotOffered {
            choice: "c99".to_string(),
        };
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, r#"{"not_offered":{"choice":"c99"}}"#);
        assert_eq!(
            serde_json::from_str::<HandBackReason>(&json).unwrap(),
            reason
        );
    }
}
