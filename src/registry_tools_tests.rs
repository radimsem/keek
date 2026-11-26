use super::*;
use crate::config::DecisionModelConfig;
use crate::decision_model::{DecisionFuture, DecisionModel, DecisionRequest};
use crate::session::delegate::Delegate;

struct Silent;

impl DecisionModel for Silent {
    fn decide(&self, _: DecisionRequest) -> DecisionFuture<'_> {
        Box::pin(std::future::pending())
    }
}

const KEY_VARIABLE: &str = "KEEK_TEST_JEV_KEY_VARIABLE";

fn delegate() -> Delegate {
    Delegate {
        model: Arc::new(Silent),
        config: DecisionModelConfig {
            endpoint: "https://jev.example.test".into(),
            model: "jev-test".into(),
            api_key_env: Some(KEY_VARIABLE.into()),
            threshold: 0.8,
            timeout_ms: 1500,
        },
    }
}

async fn listed(h: &mut Harness) -> Vec<Value> {
    match h.request(Request::ListTools).await {
        Response::Tools(value) => value["tools"].as_array().unwrap().clone(),
        other => panic!("expected the tool list, got {other:?}"),
    }
}

fn names(tools: &[Value]) -> Vec<&str> {
    tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn delegate_step_is_listed_only_when_a_decision_model_is_configured() {
    let mut without = Harness::new(1);
    let mut with = Harness::with_delegate(1, Some(delegate()));

    assert_eq!(names(&listed(&mut without).await), ["click", "slow"]);
    assert_eq!(
        names(&listed(&mut with).await),
        ["click", "slow", "delegate_step"]
    );
}

#[tokio::test]
async fn delegate_step_requires_a_session_an_intent_an_action_and_a_window_with_its_pid() {
    let mut h = Harness::with_delegate(1, Some(delegate()));
    let tools = listed(&mut h).await;
    let schema = &tools.last().unwrap()["inputSchema"];

    assert_eq!(
        schema["required"],
        json!(["session", "intent", "action", "window"])
    );
    assert_eq!(schema["properties"]["window"]["required"], json!(["pid"]));
    assert_eq!(
        schema["properties"]["action"]["enum"],
        json!(["click", "double_click", "right_click", "type", "select"])
    );
}

#[tokio::test]
async fn a_session_reaches_the_decision_model_only_when_one_is_configured() {
    let refusal_to = |response| refused(response);
    let mut without = Harness::new(1);
    let session = without.open(without.worktree("one")).await;
    let off = refusal_to(
        without
            .request(Request::CallTool {
                session,
                tool: "delegate_step".into(),
                arguments: json!({"intent":"click Sign in"}),
            })
            .await,
    );
    let mut with = Harness::with_delegate(1, Some(delegate()));
    let session = with.open(with.worktree("one")).await;
    let on = refusal_to(
        with.request(Request::CallTool {
            session,
            tool: "delegate_step".into(),
            arguments: json!({"intent":"click Sign in"}),
        })
        .await,
    );

    assert!(off.contains("delegate_step is off"), "{off}");
    assert!(!on.contains("delegate_step is off"), "{on}");
}

async fn status_json(h: &mut Harness) -> Value {
    match h.request(Request::Status).await {
        Response::Status(status) => serde_json::to_value(status).unwrap(),
        other => panic!("expected the status, got {other:?}"),
    }
}

#[tokio::test]
async fn keek_status_shows_the_decision_model_but_never_its_key() {
    let mut without = Harness::new(1);
    let mut with = Harness::with_delegate(1, Some(delegate()));

    let shown = status_json(&mut with).await;
    assert_eq!(
        shown["decision_model"],
        json!({"endpoint":"https://jev.example.test","model":"jev-test","threshold":0.8})
    );
    assert!(!shown.to_string().contains(KEY_VARIABLE), "{shown}");
    assert!(
        status_json(&mut without)
            .await
            .get("decision_model")
            .is_none()
    );
}
