use super::*;

const LINE: &str = "keek: Checks not marked yet: 1: fixture works. Call mark_check for each one you have already observed.";

async fn forwarded(actor: &RunningActor) -> Value {
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "click".into(),
            arguments: json!({"session":"keek-name","refuse_with":"fine","non_error":true}),
            reply,
        })
        .await
        .unwrap();
    let Response::ToolResult(value) = response.await.unwrap() else {
        panic!("call refused")
    };
    value
}
fn reminder(value: &Value) -> Option<String> {
    let content = value["content"].as_array().unwrap();
    (content.len() > 1).then(|| content.last().unwrap()["text"].as_str().unwrap().to_owned())
}
/// The 1-based numbers of the calls whose result carried a reminder.
async fn reminded_calls(actor: &RunningActor, calls: usize) -> Vec<usize> {
    let mut reminded = vec![];
    for number in 1..=calls {
        if reminder(&forwarded(actor).await).is_some() {
            reminded.push(number);
        }
    }
    reminded
}
async fn add_check(actor: &RunningActor, text: &str) {
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::AddCheck {
            text: text.into(),
            reply,
        })
        .await
        .unwrap();
    assert!(matches!(
        response.await.unwrap(),
        Response::CheckAdded { .. }
    ));
}
async fn mark_check(actor: &RunningActor, check: u32) -> Response {
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::MarkCheck {
            check: crate::timeline::CheckId(check),
            outcome: crate::timeline::CheckOutcome::Passed,
            comment: String::new(),
            reply,
        })
        .await
        .unwrap();
    response.await.unwrap()
}

#[tokio::test]
async fn the_call_after_eight_with_a_check_unmarked_ends_with_one_line_naming_it() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    assert_eq!(reminded_calls(&actor, 8).await, Vec::<usize>::new());
    assert_eq!(
        forwarded(&actor).await,
        json!({"isError":false,"content":[
            {"type":"text","text":"fine"},
            {"type":"text","text":LINE}
        ]})
    );
    finish(actor).await;
}
#[tokio::test]
async fn the_reminder_repeats_every_eight_calls() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    assert_eq!(reminded_calls(&actor, 26).await, [9, 17, 25]);
    finish(actor).await;
}
#[tokio::test]
async fn the_reminder_stops_once_no_check_is_unmarked() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    reminded_calls(&actor, 8).await;
    assert!(matches!(
        mark_check(&actor, 1).await,
        Response::CheckMarked { .. }
    ));
    assert_eq!(reminded_calls(&actor, 30).await, Vec::<usize>::new());
    finish(actor).await;
}
#[tokio::test]
async fn adding_a_check_restarts_the_count_and_the_reminder_names_every_unmarked_check() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    reminded_calls(&actor, 8).await;
    add_check(&actor, "second\nworks").await;
    assert_eq!(reminded_calls(&actor, 8).await, Vec::<usize>::new());
    assert_eq!(
        reminder(&forwarded(&actor).await).unwrap(),
        "keek: Checks not marked yet: 1: fixture works, 2: second works. Call mark_check for each one you have already observed."
    );
    mark_check(&actor, 2).await;
    finish(actor).await;
}
#[tokio::test]
async fn marking_a_check_restarts_the_count_and_the_reminder_names_only_what_is_left() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    add_check(&actor, "second works").await;
    reminded_calls(&actor, 5).await;
    assert!(matches!(
        mark_check(&actor, 1).await,
        Response::CheckMarked { .. }
    ));
    assert_eq!(reminded_calls(&actor, 8).await, Vec::<usize>::new());
    assert_eq!(
        reminder(&forwarded(&actor).await).unwrap(),
        "keek: Checks not marked yet: 2: second works. Call mark_check for each one you have already observed."
    );
    mark_check(&actor, 2).await;
    finish(actor).await;
}
#[tokio::test]
async fn a_refused_mark_does_not_restart_the_count() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    reminded_calls(&actor, 8).await;
    assert!(matches!(
        mark_check(&actor, 9).await,
        Response::Refused { .. }
    ));
    assert!(reminder(&forwarded(&actor).await).is_some());
    finish(actor).await;
}
#[tokio::test]
async fn the_footer_shows_the_reminder_until_the_next_call() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    reminded_calls(&actor, 8).await;
    assert!(!actor.events.borrow().reminded);
    forwarded(&actor).await;
    assert!(actor.events.borrow().reminded);
    forwarded(&actor).await;
    assert!(!actor.events.borrow().reminded);
    finish(actor).await;
}
#[tokio::test]
async fn each_reminder_is_in_the_timeline_with_its_time_and_not_among_the_calls() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    reminded_calls(&actor, 8).await;
    actor.clock.elapsed.fetch_add(1000, Ordering::SeqCst);
    forwarded(&actor).await;
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused")
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let calls: Vec<_> = timeline
        .events()
        .iter()
        .filter_map(|event| match event {
            TimelineEvent::Call { end_ms, .. } => Some(*end_ms),
            _ => None,
        })
        .collect();
    let reminders: Vec<_> = timeline
        .events()
        .iter()
        .filter_map(|event| match event {
            TimelineEvent::Reminder { at_ms } => Some(*at_ms),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 9);
    assert_eq!(reminders, [calls[8]]);
    assert!(calls[8] > calls[7]);
}
