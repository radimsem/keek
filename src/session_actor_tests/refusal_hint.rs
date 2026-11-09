use super::*;

const PID: &str = "Missing required integer field: pid";
const NO_WINDOWS: &str = "No windows found for pid 158. Provide window_id.";
const CAPTURE: &str = "capture-bound click refused: capture id is unknown";

async fn call_with(actor: &RunningActor, tool: &str, mut args: Value) -> Value {
    args["session"] = json!("keek-name");
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: tool.into(),
            arguments: args,
            reply,
        })
        .await
        .unwrap();
    let Response::ToolResult(value) = response.await.unwrap() else {
        panic!("call refused")
    };
    value
}
fn texts(value: &Value) -> Vec<String> {
    value["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["text"].as_str().map(str::to_owned))
        .collect()
}
fn original(text: &str) -> Value {
    json!({"isError":true,"content":[{"type":"text","text":text}]})
}
fn assert_lists_windows(value: &Value) {
    let all = texts(value).join(" ");
    assert!(all.contains("keek: Cua Driver refused this call because the pid or window_id is missing or matches no window."));
    assert!(all.contains("Call again with the pid and window_id of the window you mean."));
    for text in [
        "pid=158",
        "window_id=7",
        "Sign in",
        "pid=212",
        "window_id=9",
    ] {
        assert!(all.contains(text), "missing {text}: {all}");
    }
}
#[tokio::test]
async fn a_click_with_a_window_id_and_no_pid_keeps_cuas_text_and_adds_each_windows_pid_window_id_and_title()
 {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let value = call_with(&actor, "click", json!({"window_id":7,"refuse_with":PID})).await;
    assert_eq!(texts(&value)[0], PID);
    assert_lists_windows(&value);
    finish(actor).await;
}
#[tokio::test]
async fn get_window_state_for_a_pid_that_owns_no_window_gets_the_same_list() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let value = call_with(
        &actor,
        "get_window_state",
        json!({"pid":158,"refuse_with":NO_WINDOWS}),
    )
    .await;
    assert_lists_windows(&value);
    finish(actor).await;
}
#[tokio::test]
async fn a_click_with_an_unknown_capture_id_adds_the_instruction_to_call_get_window_state_again() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let value = call_with(&actor, "click", json!({"refuse_with":CAPTURE})).await;
    let text = texts(&value).join(" ");
    assert!(text.contains("get_window_state") && text.contains("capture_id"));
    assert!(
        !actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "list_windows")
    );
    finish(actor).await;
}
#[tokio::test]
async fn a_call_that_succeeds_makes_no_extra_driver_call() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let count = actor.calls.lock().unwrap().len();
    call_with(&actor, "click", json!({})).await;
    assert_eq!(actor.calls.lock().unwrap().len(), count + 1);
    finish(actor).await;
}
#[tokio::test]
async fn an_unrelated_refusal_reaches_the_verifier_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    for text in ["surface_identity_unproven", "Missing integer field: pid"] {
        assert_eq!(
            call_with(&actor, "get_window_state", json!({"refuse_with":text})).await,
            original(text)
        );
    }
    assert!(
        !actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "list_windows")
    );
    finish(actor).await;
}
#[tokio::test]
async fn a_refusal_text_in_a_reply_that_is_not_an_error_is_left_alone() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let value = call_with(&actor, "click", json!({"refuse_with":PID,"non_error":true})).await;
    assert_eq!(
        value,
        json!({"isError":false,"content":[{"type":"text","text":PID}]})
    );
    assert!(
        !actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "list_windows")
    );
    finish(actor).await;
}
#[tokio::test]
async fn a_pid_refusal_is_returned_as_cua_sent_it_when_list_windows_fails() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    for mode in [
        fake_driver::WindowsReply::Failed,
        fake_driver::WindowsReply::ReportedFailure,
    ] {
        *actor.windows.lock().unwrap() = mode;
        assert_eq!(
            call_with(&actor, "click", json!({"refuse_with":PID})).await,
            original(PID)
        );
    }
    finish(actor).await;
}
#[tokio::test]
async fn the_extra_list_windows_call_never_enters_the_session_timeline() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    call_with(&actor, "click", json!({"refuse_with":PID})).await;
    let calls = actor.calls.clone();
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused")
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let entries: Vec<_> = timeline
        .events()
        .iter()
        .filter(|event| matches!(event, TimelineEvent::Call { .. }))
        .collect();
    assert_eq!(entries.len(), 1);
    assert!(
        matches!(entries[0], TimelineEvent::Call { tool, is_error: true, .. } if tool == "click")
    );
    let calls = calls.lock().unwrap();
    let names: Vec<_> = calls.iter().map(|(tool, _)| tool.as_str()).collect();
    assert_eq!(
        names.iter().filter(|tool| **tool == "list_windows").count(),
        1
    );
    assert!(
        names.iter().position(|tool| *tool == "click")
            < names.iter().position(|tool| *tool == "list_windows")
    );
}
#[tokio::test]
async fn a_test_credential_value_in_a_window_title_is_masked_in_the_hint() {
    let root = tempfile::tempdir().unwrap();
    let credentials =
        crate::credentials::Credentials::parse(Path::new("fixture.env"), "PASSWORD=made-up-value")
            .unwrap();
    let (actor, _) =
        start_actor_with_credentials(root.path(), "app_12-00-00", false, None, credentials).await;
    let value = call_with(&actor, "click", json!({"refuse_with":PID}))
        .await
        .to_string();
    assert!(!value.contains("made-up-value"));
    assert!(value.contains("[masked]"));
    finish(actor).await;
}
