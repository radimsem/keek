use super::*;
const VALUE: &str = "made-up-value";
async fn start(root: &Path) -> (RunningActor, Response) {
    start_actor_with_credentials(
        root,
        "app_12-00-00",
        false,
        None,
        crate::credentials::Credentials::parse(Path::new("fixture.env"), "PASSWORD=made-up-value")
            .unwrap(),
    )
    .await
}
async fn argument(actor: &RunningActor, args: Value) -> Response {
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            tool: "run_steps".into(),
            arguments: args,
            reply,
        })
        .await
        .unwrap();
    response.await.unwrap()
}
#[tokio::test]
async fn opened_carries_only_credential_names() {
    let root = tempfile::tempdir().unwrap();
    let (actor, response) = start(root.path()).await;
    let Response::Opened { credentials, .. } = response else {
        panic!("open refused")
    };
    assert_eq!(credentials, ["PASSWORD"]);
    finish(actor).await;
}
#[tokio::test]
async fn driver_receives_substituted_nested_arguments_and_replies_are_masked() {
    let root = tempfile::tempdir().unwrap();
    let (actor, _) = start(root.path()).await;
    let response = argument(
        &actor,
        json!({"steps":[{"text":"${PASSWORD}"}],"echo":true}),
    )
    .await;
    assert!(actor.calls.lock().unwrap().last().unwrap().1["steps"][0]["text"] == VALUE);
    let Response::ToolResult(v) = response else {
        panic!("call refused")
    };
    assert!(!v.to_string().contains(VALUE));
    assert!(v.to_string().contains("[masked]"));
    finish(actor).await;
}
#[tokio::test]
async fn echoed_content_and_structured_content_are_masked() {
    let root = tempfile::tempdir().unwrap();
    let (actor, _) = start(root.path()).await;
    let Response::ToolResult(value) = argument(&actor, json!({"text":VALUE,"echo":true})).await
    else {
        panic!("call refused")
    };
    assert!(
        !value["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(VALUE)
    );
    assert_eq!(value["structuredContent"]["text"], "[masked]");
    finish(actor).await;
}
#[tokio::test]
async fn unknown_credentials_never_reach_driver_or_timeline() {
    let root = tempfile::tempdir().unwrap();
    let (actor, _) = start(root.path()).await;
    let count = actor.calls.lock().unwrap().len();
    assert!(matches!(
        argument(&actor, json!({"text":"${MISSING}"})).await,
        Response::Refused { .. }
    ));
    assert_eq!(actor.calls.lock().unwrap().len(), count);
    let Response::Closed(v) = finish(actor).await else {
        panic!("close refused")
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(v["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(timeline.events().len(), 1);
}
#[tokio::test]
async fn echoed_driver_errors_are_masked() {
    let root = tempfile::tempdir().unwrap();
    let (actor, _) = start(root.path()).await;
    let response = argument(&actor, json!({"text":VALUE,"echo_error":true})).await;
    let Response::Refused { message } = response else {
        panic!("call succeeded")
    };
    assert!(!message.contains(VALUE));
    assert!(message.contains("[masked]"));
    finish(actor).await;
}
#[tokio::test]
async fn checks_comments_and_summary_are_masked_in_snapshots_and_reports() {
    let root = tempfile::tempdir().unwrap();
    let (actor, _) = start(root.path()).await;
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::AddCheck {
            text: VALUE.into(),
            reply,
        })
        .await
        .unwrap();
    let Response::CheckAdded { check } = response.await.unwrap() else {
        panic!("add refused")
    };
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::MarkCheck {
            check,
            outcome: crate::timeline::CheckOutcome::Passed,
            comment: VALUE.into(),
            reply,
        })
        .await
        .unwrap();
    response.await.unwrap();
    assert_eq!(actor.events.borrow().checks[1].text, "[masked]");
    assert!(!format!("{:?}", actor.events.borrow()).contains(VALUE));
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::MarkCheck {
            check: CheckId(1),
            outcome: CheckOutcome::Passed,
            comment: VALUE.into(),
            reply,
        })
        .await
        .unwrap();
    response.await.unwrap();
    let folder = actor.folder.root().to_owned();
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Close {
            summary: VALUE.into(),
            reply,
        })
        .await
        .unwrap();
    let ended = actor.task.await.unwrap();
    ended.reply.unwrap().send(ended.response).unwrap();
    let Response::Closed(v) = response.await.unwrap() else {
        panic!("close refused")
    };
    let report = std::fs::read_to_string(v["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(!report.contains(VALUE));
    assert!(!folder.exists());
}
