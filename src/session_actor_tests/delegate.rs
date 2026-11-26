use super::close_tests::{finished, mark, request_close};
use super::fake_decision_model::{FakeDecisionModel, pick};
use super::*;
use crate::config::DecisionModelConfig;
use crate::credentials::Credentials;
use crate::step_policy::HandBackReason;
use crate::timeline::{CandidateSummary, DelegatedOutcome, DelegatedStepEvent};

const SECRET: &str = "made-up-secret-1";
const SETUP_CALLS: usize = 3;

async fn start(
    model: &Arc<FakeDecisionModel>,
    timeout_ms: u64,
) -> (tempfile::TempDir, RunningActor) {
    let root = tempfile::tempdir().unwrap();
    let credentials =
        Credentials::parse(Path::new("fixture.env"), &format!("TEST_PASSWORD={SECRET}")).unwrap();
    let delegate = delegate::Delegate {
        model: model.clone(),
        config: DecisionModelConfig {
            endpoint: "http://127.0.0.1:9".into(),
            model: "jev-fixture".into(),
            api_key_env: None,
            threshold: 0.9,
            timeout_ms,
        },
    };
    let (actor, response) = start_with(
        root.path(),
        "app_12-00-00",
        false,
        None,
        credentials,
        Duration::from_secs(300),
        None,
        Some(delegate),
    )
    .await;
    assert!(matches!(response, Response::Opened { .. }), "{response:?}");
    (root, actor)
}

async fn send(actor: &RunningActor, tool: &str, arguments: Value) -> Response {
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: tool.into(),
            arguments,
            reply,
        })
        .await
        .unwrap();
    response.await.unwrap()
}

fn step(intent: &str, action: &str) -> Value {
    json!({"session":"app_12-00-00","intent":intent,"action":action,"window":{"pid":4242,"window_id":7}})
}

fn driven(actor: &RunningActor) -> Vec<(String, Value)> {
    actor.calls.lock().unwrap()[SETUP_CALLS..].to_vec()
}

fn text(response: &Response) -> String {
    let Response::ToolResult(value) = response else {
        panic!("step refused: {response:?}");
    };
    value["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Closes the session and returns its filed timeline, as text and as events.
async fn filed_timeline(actor: RunningActor) -> (String, Vec<TimelineEvent>) {
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused");
    };
    let text = std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap();
    let events = SessionTimeline::parse(&text).unwrap().events().to_vec();
    (text, events)
}

fn delegated(events: &[TimelineEvent]) -> Vec<DelegatedStepEvent> {
    events
        .iter()
        .filter_map(|event| {
            if let TimelineEvent::DelegatedStep(step) = event {
                Some(step.clone())
            } else {
                None
            }
        })
        .collect()
}

#[tokio::test]
async fn a_confident_step_clicks_the_candidates_element_token() {
    let model = Arc::new(FakeDecisionModel::answering(vec![Ok(pick(
        "c1",
        &[("c1", 0.95), ("c2", 0.03)],
        0.97,
    ))]));
    let (_root, actor) = start(&model, 1500).await;
    let Response::ToolResult(value) =
        send(&actor, "delegate_step", step("click Sign in", "click")).await
    else {
        panic!("step refused");
    };
    assert_eq!(
        driven(&actor),
        [
            ("get_window_state".into(), json!({"pid":4242,"window_id":7})),
            (
                "click".into(),
                json!({"pid":4242,"element_token":"s00000008:39"})
            ),
        ]
    );
    assert_eq!(
        value["structuredContent"]["delegated_step"],
        json!({"pick":"c1","probability":0.95,"decision_ms":12})
    );
    finish(actor).await;
}

#[tokio::test]
async fn a_pick_cua_refuses_says_keek_tried_it_and_counts_as_an_action() {
    let model = Arc::new(FakeDecisionModel::answering(vec![Ok(pick(
        "c1",
        &[("c1", 0.95)],
        0.97,
    ))]));
    let (_root, actor) = start(&model, 1500).await;
    {
        let mut state = actor.window_state.lock().unwrap();
        for element in state["structuredContent"]["elements"]
            .as_array_mut()
            .unwrap()
        {
            if element["label"] == "Sign in" {
                element["element_token"] = json!(fake_driver::REFUSED_TOKEN);
            }
        }
    }
    let response = send(&actor, "delegate_step", step("click Sign in", "click")).await;
    assert_eq!(
        text(&response),
        "fixture refusal\nkeek tried c1 button \"Sign in\" 0.95 after a 12 ms decision, and Cua refused."
    );
    let (_, events) = filed_timeline(actor).await;
    assert!(events.iter().any(|event| matches!(
        event,
        TimelineEvent::Call { tool, action: true, is_error: true, .. } if tool == "delegate_step"
    )));
    let [step] = delegated(&events).try_into().unwrap();
    assert_eq!(step.outcome, DelegatedOutcome::Acted);
}

#[tokio::test]
async fn a_confident_select_sets_the_option_by_name() {
    let model = Arc::new(FakeDecisionModel::answering(vec![Ok(pick(
        "c1",
        &[("c1", 0.96)],
        0.97,
    ))]));
    let (_root, actor) = start(&model, 1500).await;
    let mut args = step("select Deutsch in Language", "select");
    args["option"] = json!("Deutsch");
    send(&actor, "delegate_step", args).await;
    assert_eq!(
        driven(&actor)[1],
        (
            "set_value".into(),
            json!({"pid":4242,"value":"Deutsch","element_token":"s00000008:23"})
        )
    );
    finish(actor).await;
}

#[tokio::test]
async fn a_step_runs_get_window_state_once() {
    let model = Arc::new(FakeDecisionModel::answering(vec![
        Ok(pick("abstain", &[("abstain", 0.99)], 0.99)),
        Ok(pick("c1", &[("c1", 0.95)], 0.97)),
    ]));
    let (_root, actor) = start(&model, 1500).await;
    let observed = |actor: &RunningActor| {
        driven(actor)
            .iter()
            .filter(|(tool, _)| tool == "get_window_state")
            .count()
    };
    send(&actor, "delegate_step", step("click Sign in", "click")).await;
    assert_eq!(observed(&actor), 1);
    send(&actor, "delegate_step", step("click Sign in", "click")).await;
    assert_eq!(observed(&actor), 2);
    finish(actor).await;
}

#[tokio::test]
async fn a_handed_back_step_does_not_act_and_lists_the_top_candidates() {
    let probabilities = [
        ("c1", 0.41),
        ("c2", 0.05),
        ("c3", 0.30),
        ("c4", 0.10),
        ("c5", 0.04),
        ("c6", 0.06),
        ("c7", 0.02),
    ];
    let model = Arc::new(FakeDecisionModel::answering(vec![Ok(pick(
        "c1",
        &probabilities,
        0.95,
    ))]));
    let (_root, actor) = start(&model, 1500).await;
    let response = send(&actor, "delegate_step", step("click Sign in", "click")).await;
    assert_eq!(
        driven(&actor),
        [("get_window_state".into(), json!({"pid":4242,"window_id":7}))]
    );
    assert_eq!(
        text(&response),
        concat!(
            "No action taken: the best pick had probability 0.41, below the threshold of 0.90. ",
            "Act on the window yourself. Top candidates: c1 button \"Sign in\" 0.41, ",
            "c3 password text \"Password\" 0.30, c4 combo box \"Language\" 0.10, ",
            "c6 menu item \"Deutsch\" 0.06, c2 entry \"Email address\" 0.05."
        )
    );
    finish(actor).await;
}

#[tokio::test]
async fn a_secret_never_reaches_the_decision_model() {
    let model = Arc::new(FakeDecisionModel::answering(vec![Ok(pick(
        "c1",
        &[("c1", 0.95)],
        0.97,
    ))]));
    let (_root, actor) = start(&model, 1500).await;
    {
        let mut state = actor.window_state.lock().unwrap();
        state["structuredContent"]["window_title"] = json!(format!("Example - {SECRET}"));
        state["structuredContent"]["elements"][1]["value"] = json!(SECRET);
    }
    let mut args = step("type ${TEST_PASSWORD} into Password", "type");
    args["text"] = json!("${TEST_PASSWORD}");
    let response = send(&actor, "delegate_step", args).await;
    let requests = format!("{:?}", model.requests.lock().unwrap());
    assert!(!requests.contains(SECRET), "{requests}");
    assert!(requests.contains("type ${TEST_PASSWORD} into Password"));
    assert_eq!(
        driven(&actor)[1],
        (
            "type_text".into(),
            json!({"pid":4242,"text":SECRET,"element_token":"s00000008:17"})
        )
    );
    assert!(!format!("{response:?}").contains(SECRET));
    finish(actor).await;
}

#[tokio::test(start_paused = true)]
async fn a_model_timeout_hands_back_within_the_limit() {
    let model = Arc::new(FakeDecisionModel::never());
    let (_root, actor) = start(&model, 200).await;
    let response = tokio::time::timeout(
        Duration::from_millis(250),
        send(&actor, "delegate_step", step("click Sign in", "click")),
    )
    .await
    .unwrap();
    assert!(
        text(&response).starts_with(
            "No action taken: the Decision Model did not answer within 200 ms. Act on the window yourself."
        ),
        "{response:?}"
    );
    assert!(matches!(
        send(
            &actor,
            "get_window_state",
            json!({"pid":4242,"window_id":7})
        )
        .await,
        Response::ToolResult(_)
    ));
    let (_, events) = filed_timeline(actor).await;
    let [step] = delegated(&events).try_into().unwrap();
    assert!(matches!(
        step.outcome,
        DelegatedOutcome::HandedBack(HandBackReason::ModelFailed(_))
    ));
    assert_eq!(step.latency_ms, 200);
}

#[tokio::test(start_paused = true)]
async fn a_close_stops_a_step_whose_decision_model_never_answers() {
    let model = Arc::new(FakeDecisionModel::never());
    let (_root, actor) = start(&model, 60_000).await;
    mark(&actor, 1).await.await.unwrap();
    let (reply, stepped) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "delegate_step".into(),
            arguments: step("click Sign in", "click"),
            reply,
        })
        .await
        .unwrap();
    while model.requests.lock().unwrap().is_empty() {
        tokio::task::yield_now().await;
    }
    let answer = request_close(&actor).await;
    let start = tokio::time::Instant::now();
    let value = finished(actor, answer).await;
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "close waited for the Decision Model"
    );
    let Response::Refused { message } = stepped.await.unwrap() else {
        panic!("step not refused");
    };
    assert_eq!(
        message,
        "Agent Session app_12-00-00 is closing, so delegate_step was stopped. Open a new Agent Session."
    );
    let text = std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap();
    let events = SessionTimeline::parse(&text).unwrap().events().to_vec();
    assert!(events.iter().any(|event| matches!(
        event,
        TimelineEvent::Call { tool, is_error: true, .. } if tool == "delegate_step"
    )));
}

#[tokio::test]
async fn every_step_adds_one_delegated_step_event() {
    let model = Arc::new(FakeDecisionModel::answering(vec![
        Ok(pick("c1", &[("c1", 0.95), ("c2", 0.05)], 0.97)),
        Ok(pick("abstain", &[("abstain", 0.99)], 0.99)),
    ]));
    let (_root, actor) = start(&model, 1500).await;
    let mut args = step("type ${TEST_PASSWORD} into Password", "type");
    args["text"] = json!("${TEST_PASSWORD}");
    send(&actor, "delegate_step", args).await;
    send(&actor, "delegate_step", step("click Sign in", "click")).await;
    let (text, events) = filed_timeline(actor).await;
    assert!(!text.contains(SECRET), "{text}");
    assert!(!text.contains("s00000008:"), "{text}");
    let calls: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let TimelineEvent::Call { tool, action, .. } = event {
                Some((tool.as_str(), *action))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls, [("delegate_step", true), ("delegate_step", false)]);
    let [typed, clicked] = delegated(&events).try_into().unwrap();
    assert_eq!(typed.intent, "type ${TEST_PASSWORD} into Password");
    assert_eq!(typed.outcome, DelegatedOutcome::Acted);
    assert_eq!(typed.pick.as_deref(), Some("c1"));
    assert_eq!(
        typed.candidates[0],
        CandidateSummary {
            id: "c1".into(),
            role: "password text".into(),
            label: Some("Password".into()),
        }
    );
    assert_eq!(
        (typed.model.as_str(), typed.threshold, typed.latency_ms),
        ("jev-fixture", 0.9, 12)
    );
    assert_eq!(
        clicked.outcome,
        DelegatedOutcome::HandedBack(HandBackReason::Abstained)
    );
}

#[tokio::test]
async fn an_unknown_credential_name_is_refused_before_the_model_is_asked() {
    let model = Arc::new(FakeDecisionModel::answering(vec![]));
    let (_root, actor) = start(&model, 1500).await;
    let mut in_intent = step("type ${UNKNOWN_NAME} into Password", "type");
    in_intent["text"] = json!("plain text");
    let mut in_text = step("type into Password", "type");
    in_text["text"] = json!("${UNKNOWN_NAME}");
    for args in [in_intent, in_text] {
        let response = send(&actor, "delegate_step", args).await;
        let Response::Refused { message } = response else {
            panic!("step not refused: {response:?}");
        };
        assert!(
            message.contains("Unknown Test Credential UNKNOWN_NAME"),
            "{message}"
        );
    }
    assert!(model.requests.lock().unwrap().is_empty());
    assert!(driven(&actor).is_empty());
    finish(actor).await;
}

#[tokio::test]
async fn a_step_without_a_decision_model_is_refused_with_what_to_configure() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let response = send(&actor, "delegate_step", step("click Sign in", "click")).await;
    let Response::Refused { message } = response else {
        panic!("step not refused: {response:?}");
    };
    assert!(message.contains("[decision_model]"), "{message}");
    assert!(message.contains("api_key_env"), "{message}");
    assert!(message.contains("restart the keek daemon"), "{message}");
    assert!(driven(&actor).is_empty());
    finish(actor).await;
}

#[tokio::test]
async fn a_step_where_nothing_matches_hands_back_with_the_top_candidates() {
    let model = Arc::new(FakeDecisionModel::answering(vec![Ok(pick(
        "c1",
        &[("c1", 0.95)],
        0.10,
    ))]));
    let (_root, actor) = start(&model, 1500).await;
    let response = send(&actor, "delegate_step", step("click Sign in", "click")).await;
    assert_eq!(
        driven(&actor),
        [("get_window_state".into(), json!({"pid":4242,"window_id":7}))]
    );
    let text = text(&response);
    assert!(
        text.starts_with(concat!(
            "No action taken: the Decision Model gave 0.10 that any candidate matches the intent, ",
            "below the threshold of 0.90. Act on the window yourself. ",
            "Top candidates: c1 button \"Sign in\" 0.95, c2 entry \"Email address\","
        )),
        "{text}"
    );
    let (_, events) = filed_timeline(actor).await;
    let [step] = delegated(&events).try_into().unwrap();
    assert_eq!(
        step.outcome,
        DelegatedOutcome::HandedBack(HandBackReason::NoMatch { matches: 0.10 })
    );
}

#[tokio::test]
async fn a_window_with_no_candidates_for_the_action_hands_back_without_asking_the_model() {
    let model = Arc::new(FakeDecisionModel::answering(vec![]));
    let (_root, actor) = start(&model, 1500).await;
    {
        let mut state = actor.window_state.lock().unwrap();
        let elements = state["structuredContent"]["elements"]
            .as_array_mut()
            .unwrap();
        elements.retain(|element| element["role"] == "frame" || element["role"] == "static");
    }
    let response = send(&actor, "delegate_step", step("click Sign in", "click")).await;
    assert_eq!(
        driven(&actor),
        [("get_window_state".into(), json!({"pid":4242,"window_id":7}))]
    );
    assert_eq!(
        text(&response),
        "No action taken: nothing on screen fits a click step. Act on the window yourself. Top candidates: none."
    );
    let Response::ToolResult(value) = &response else {
        panic!("step refused: {response:?}");
    };
    assert_eq!(
        value["structuredContent"]["delegated_step"]["reason"],
        "no_candidates"
    );
    assert!(model.requests.lock().unwrap().is_empty());
    let (_, events) = filed_timeline(actor).await;
    let [step] = delegated(&events).try_into().unwrap();
    assert_eq!(
        step.outcome,
        DelegatedOutcome::HandedBack(HandBackReason::NoCandidates)
    );
    assert_eq!((step.pick, step.latency_ms), (None, 0));
}
