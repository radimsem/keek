use serde_json::{Value, json};

use super::{CANDIDATE_CAP, Candidate, CandidatesError, Frame, StepAction, candidates};

fn sign_in_state() -> Value {
    serde_json::from_str(include_str!("../tests/fixtures/window_state_signin.json")).unwrap()
}

fn labels(found: &[Candidate]) -> Vec<&str> {
    found
        .iter()
        .map(|candidate| candidate.label.as_deref().unwrap_or(""))
        .collect()
}

fn element(role: &str, label: &str, token: &str) -> Value {
    json!({
        "role": role,
        "label": label,
        "element_token": token,
        "frame": { "x": 1.0, "y": 2.0, "w": 30.0, "h": 4.0 },
    })
}

fn state_of(elements: Vec<Value>) -> Value {
    json!({ "structuredContent": { "elements": elements } })
}

#[test]
fn click_candidates_are_enabled_elements_with_a_token_and_a_frame() {
    let found = candidates(&sign_in_state(), StepAction::Click, "go").unwrap();

    assert_eq!(
        labels(&found),
        [
            "Email address",
            "Password",
            "Language",
            "English",
            "Deutsch",
            "Forgot password?",
            "Sign in",
        ]
    );
}

#[test]
fn web_content_elements_are_offered() {
    let found = candidates(&sign_in_state(), StepAction::Click, "click the email field").unwrap();

    let email = found
        .iter()
        .find(|candidate| candidate.label.as_deref() == Some("Email address"))
        .unwrap();
    assert!(email.in_web_content);
    assert_eq!(email.role, "entry");
    assert_eq!(email.element_token, "s00000008:12");
    assert_eq!(
        email.frame,
        Frame {
            x: 420.0,
            y: 240.0,
            w: 440.0,
            h: 40.0
        }
    );
}

#[test]
fn type_candidates_are_text_entries_only() {
    let found = candidates(&sign_in_state(), StepAction::Type, "type the address").unwrap();

    assert_eq!(labels(&found), ["Email address", "Password"]);
    assert_eq!(found[1].role, "password text");
}

#[test]
fn select_candidates_are_combo_boxes_and_lists() {
    let state = state_of(vec![
        element("button", "Apply", "s1:1"),
        element("combo box", "Language", "s1:2"),
        element("list", "Recent files", "s1:3"),
        element("entry", "Search", "s1:4"),
    ]);

    let found = candidates(&state, StepAction::Select, "choose").unwrap();

    assert_eq!(labels(&found), ["Language", "Recent files"]);
}

#[test]
fn candidates_matching_the_intent_come_first() {
    let found = candidates(&sign_in_state(), StepAction::Click, "click Sign in").unwrap();

    assert_eq!(found[0].id, "c1");
    assert_eq!(found[0].label.as_deref(), Some("Sign in"));
    assert_eq!(found[0].element_token, "s00000008:39");
    let ids: Vec<&str> = found
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    assert_eq!(ids, ["c1", "c2", "c3", "c4", "c5", "c6", "c7"]);
    assert_eq!(found[1].label.as_deref(), Some("Email address"));
}

#[test]
fn a_value_match_ranks_like_a_label_match() {
    let state = state_of(vec![
        json!({
            "role": "combo box", "label": "Region", "value": "Europe",
            "element_token": "s1:1", "frame": { "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 },
        }),
        json!({
            "role": "combo box", "label": "Language", "value": "Deutsch",
            "element_token": "s1:2", "frame": { "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 },
        }),
    ]);

    let found = candidates(&state, StepAction::Select, "Pick DEUTSCH").unwrap();

    assert_eq!(found[0].label.as_deref(), Some("Language"));
    assert_eq!(found[0].value.as_deref(), Some("Deutsch"));
}

#[test]
fn more_than_the_cap_keeps_the_best_matches() {
    let links = (0..300)
        .map(|index| {
            let label = if index == 280 {
                "Pricing".to_string()
            } else {
                format!("Page {index}")
            };
            element("link", &label, &format!("s1:{index}"))
        })
        .collect();

    let found = candidates(&state_of(links), StepAction::Click, "click Pricing").unwrap();

    assert_eq!(found.len(), CANDIDATE_CAP);
    assert_eq!(found[0].label.as_deref(), Some("Pricing"));
    assert_eq!(found[0].id, "c1");
    assert_eq!(found[23].id, "c24");
    assert_eq!(found[1].label.as_deref(), Some("Page 0"));
}

#[test]
fn a_state_without_elements_is_an_error() {
    let state = json!({ "structuredContent": { "pid": 1, "window_id": 2 } });

    let error = candidates(&state, StepAction::Click, "click Sign in").unwrap_err();

    assert!(matches!(error, CandidatesError::NoElements));
    assert_eq!(
        error.to_string(),
        "get_window_state returned no element list for this window. Call get_window_state and act on the window yourself."
    );
}

#[test]
fn a_bare_window_state_works_without_the_tool_result_envelope() {
    let bare = sign_in_state()["structuredContent"].clone();

    let found = candidates(&bare, StepAction::Click, "click Sign in").unwrap();

    assert_eq!(found[0].label.as_deref(), Some("Sign in"));
}

#[test]
fn step_actions_serialize_as_snake_case() {
    assert_eq!(
        serde_json::to_value(StepAction::DoubleClick).unwrap(),
        "double_click"
    );
}
