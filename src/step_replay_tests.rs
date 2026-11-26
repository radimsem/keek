use std::collections::BTreeMap;
use std::time::Duration;

use super::{Expected, Outcome, absent_label, outcome, partial_intent};
use crate::decision_model::Decision;
use crate::step_candidates::{Candidate, Frame};

const TREE: &str = r#"- [0] frame "Sample app - Browser" [actions=[doDefault,showContextMenu]]
  - [5] link "Continue with a provider" [actions=[jump,showContextMenu]]
  - [9] button "Sign in" (description "Submits the form") [actions=[press,showContextMenu]]
"#;

fn candidate(id: &str) -> Candidate {
    Candidate {
        id: id.to_owned(),
        element_token: id.to_owned(),
        role: "button".to_owned(),
        label: Some(id.to_owned()),
        value: None,
        frame: Frame {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        },
        in_web_content: false,
    }
}

fn picks(choice: &str, probability: f64) -> Decision {
    Decision {
        choice: choice.to_owned(),
        probabilities: BTreeMap::from([(choice.to_owned(), probability)]),
        matches: probability,
        latency: Duration::from_millis(1),
    }
}

#[test]
fn a_partial_intent_names_the_role_and_the_longest_word_of_the_label() {
    assert_eq!(
        partial_intent("toggle button", "Save all Changes").as_deref(),
        Some("click the changes toggle button")
    );
}

#[test]
fn a_label_with_no_word_gives_no_partial_intent() {
    assert_eq!(partial_intent("button", "× …"), None);
}

#[test]
fn an_absent_label_is_one_the_tree_holds_nowhere_in_any_case() {
    let others = ["sign IN", "Continue", "Export report", "Print"];

    assert_eq!(absent_label(TREE, others), Some("Export report"));
}

#[test]
fn no_absent_label_is_found_when_the_tree_holds_every_one() {
    assert_eq!(absent_label(TREE, ["Sign in", "provider"]), None);
}

#[test]
fn an_act_is_right_only_on_the_expected_control_and_a_low_pick_hands_back() {
    let candidates = [candidate("c1"), candidate("c2")];
    let decision = picks("c2", 0.95);

    let on = |expected: Expected, threshold| outcome(&decision, &candidates, &expected, threshold);

    assert_eq!(
        on(Expected::Control("c2".to_owned()), 0.9),
        Outcome::RightAct
    );
    assert_eq!(
        on(Expected::Control("c1".to_owned()), 0.9),
        Outcome::WrongAct
    );
    assert_eq!(on(Expected::HandBack, 0.9), Outcome::WrongAct);
    assert_eq!(on(Expected::HandBack, 0.99), Outcome::HandedBack);
}
