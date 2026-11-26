use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::{DecisionModelError, DecisionRequest, JevClient, build_request, parse_answer};
use crate::config::DecisionModelConfig;
use crate::step_candidates::{Candidate, Frame, StepAction};

const ELEMENT_TOKEN: &str = "tok-made-up-7731";

fn candidate(id: &str, role: &str, label: Option<&str>, value: Option<&str>) -> Candidate {
    Candidate {
        id: id.to_owned(),
        element_token: ELEMENT_TOKEN.to_owned(),
        role: role.to_owned(),
        label: label.map(str::to_owned),
        value: value.map(str::to_owned),
        frame: Frame {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
        },
        in_web_content: true,
    }
}

fn request() -> DecisionRequest {
    DecisionRequest {
        intent: "click Sign in".to_owned(),
        action: StepAction::Click,
        window_title: Some("Welcome back".to_owned()),
        candidates: vec![
            candidate("c1", "button", Some("Sign in"), None),
            candidate(
                "c2",
                "entry",
                Some("Email address"),
                Some("alice@example.test"),
            ),
        ],
    }
}

fn config(api_key_env: Option<&str>) -> DecisionModelConfig {
    DecisionModelConfig {
        endpoint: "http://127.0.0.1:9".to_owned(),
        model: "jev-test".to_owned(),
        api_key_env: api_key_env.map(str::to_owned),
        threshold: 0.9,
        timeout_ms: 1500,
    }
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("../tests/fixtures/jev_answer.json")).unwrap()
}

#[test]
fn the_request_has_one_choice_and_one_noul_question_over_one_state() {
    let body = build_request("jev-test", &request());

    assert_eq!(body["model"], "jev-test");
    let questions = body["questions"].as_object().unwrap();
    assert_eq!(questions.len(), 2);
    let mut types: Vec<&str> = questions
        .values()
        .map(|question| question["type"].as_str().unwrap())
        .collect();
    types.sort_unstable();
    assert_eq!(types, ["choice", "noul"]);
    assert_eq!(body["state"]["intent"], "click Sign in");
    assert_eq!(body["state"]["action"], "click");
    assert_eq!(body["state"]["window_title"], "Welcome back");
    assert_eq!(body["state"]["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(
        body["state"]["candidates"][1]["value"],
        "alice@example.test"
    );
}

#[test]
fn choice_options_are_the_candidate_ids_plus_reobserve_and_abstain() {
    let body = build_request("jev-test", &request());

    let options: BTreeSet<&str> = body["questions"]["pick"]["criteria"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        options,
        BTreeSet::from(["abstain", "c1", "c2", "reobserve"])
    );
    let sign_in = body["questions"]["pick"]["criteria"]["c1"]
        .as_str()
        .unwrap();
    assert!(sign_in.contains("button") && sign_in.contains("Sign in"));
}

#[test]
fn element_tokens_are_not_in_the_request() {
    let body = build_request("jev-test", &request());

    assert!(!body.to_string().contains(ELEMENT_TOKEN));
}

#[test]
fn parse_answer_reads_the_choice_its_probabilities_and_the_noul() {
    let (choice, probabilities, matches) = parse_answer(&fixture()).unwrap();

    assert_eq!(choice, "c1");
    assert_eq!(probabilities["c1"], 0.95);
    assert_eq!(probabilities.len(), 5);
    assert_eq!(matches, 0.98);
}

#[test]
fn parse_answer_refuses_a_body_without_the_choice() {
    let body = json!({ "answers": { "matches": { "type": "noul", "noul": 0.98 } } });

    let error = parse_answer(&body).unwrap_err();

    assert!(matches!(error, DecisionModelError::Malformed { .. }));
}

#[test]
fn a_missing_key_variable_is_an_error_naming_the_variable_not_its_value() {
    let var = "KEEK_TEST_JEV_KEY_THAT_IS_NEVER_SET";

    let error = JevClient::new(&config(Some(var))).unwrap_err();

    assert!(matches!(&error, DecisionModelError::MissingKey { var: named } if named == var));
    assert!(error.to_string().contains(var));
}

#[test]
fn a_client_without_a_key_variable_builds() {
    assert!(JevClient::new(&config(None)).is_ok());
}

#[test]
fn the_client_debug_output_hides_the_key() {
    let client = JevClient::with_key(
        &config(Some("ANY")),
        Some("sk-made-up-debug-key".to_owned()),
    )
    .unwrap();

    let shown = format!("{client:?}");

    assert!(!shown.contains("sk-made-up-debug-key"));
    assert!(shown.contains("jev-test"));
}
