use super::*;
use crate::checklist::Mark;
use crate::timeline::{CheckId, CheckOutcome, TimelineEvent};

#[tokio::test]
async fn refused_close_keeps_calls_and_check_marks_working() {
    let mut h = Harness::new(1);
    let session = h.open(h.worktree("one")).await;
    let entry = h.registry.sessions.get(&session).unwrap();
    let mut events = entry.events.clone();
    let message = refused(
        h.request(Request::CloseSession {
            session: session.clone(),
            summary: "".into(),
        })
        .await,
    );
    assert!(message.contains("1: check"), "{message}");
    assert!(matches!(
        h.request(Request::CallTool {
            session: session.clone(),
            tool: "click".into(),
            arguments: json!({})
        })
        .await,
        Response::ToolResult(_)
    ));
    let added = h
        .request(Request::AddCheck {
            session: session.clone(),
            text: "added criterion".into(),
        })
        .await;
    assert_eq!(added, Response::CheckAdded { check: CheckId(2) });
    let snapshot = events.borrow_and_update();
    assert!(
        matches!(&snapshot.checks[..], [first, second] if first.id == CheckId(1) && second.id == CheckId(2) && second.text == "added criterion" && second.mark == Mark::Unmarked)
    );
    drop(snapshot);
    assert!(matches!(
        h.request(Request::MarkCheck {
            session: session.clone(),
            check: CheckId(2),
            outcome: CheckOutcome::Failed,
            comment: " ".into()
        })
        .await,
        Response::Refused { .. }
    ));
    for outcome in [CheckOutcome::Failed, CheckOutcome::Skipped] {
        assert_eq!(
            h.request(Request::MarkCheck {
                session: session.clone(),
                check: CheckId(2),
                outcome,
                comment: "observed".into()
            })
            .await,
            Response::CheckMarked { check: CheckId(2) }
        );
        assert!(
            matches!(&events.borrow_and_update().checks[1].mark,Mark::Outcome { outcome: o, .. } if *o == outcome)
        );
    }
    let Response::Closed(value) = h.finish(session).await else {
        panic!("close refused");
    };
    assert_eq!(value["verdict"], "Partly verified");
    assert_eq!(value["checks"][1]["outcome"], "skipped");
    let timeline = crate::timeline::SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let marks: Vec<_> = timeline
        .events()
        .iter()
        .filter_map(|e| match e {
            TimelineEvent::CheckMark { check, outcome, .. } => Some((*check, *outcome)),
            TimelineEvent::Call { .. }
            | TimelineEvent::Takeover { .. }
            | TimelineEvent::Reminder { .. }
            | TimelineEvent::HelpRequest { .. } => None,
        })
        .collect();
    assert_eq!(
        marks,
        vec![
            (CheckId(2), CheckOutcome::Failed),
            (CheckId(2), CheckOutcome::Skipped),
            (CheckId(1), CheckOutcome::Passed)
        ]
    );
}

fn mark_passed(session: &SessionName) -> Request {
    Request::MarkCheck {
        session: session.clone(),
        check: CheckId(1),
        outcome: CheckOutcome::Passed,
        comment: "".into(),
    }
}

#[tokio::test]
async fn a_mark_sent_right_after_a_close_that_gets_refused_still_lands() {
    let mut h = Harness::new(1);
    let session = h.open(h.worktree("one")).await;
    let (close_reply, close_answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: session.clone(),
                summary: "".into(),
            },
            close_reply,
            None,
        )
        .await;
    assert!(refused(close_answer.await.unwrap()).contains("1: check"));
    let (mark_reply, mark_answer) = oneshot::channel();
    h.registry
        .handle(mark_passed(&session), mark_reply, None)
        .await;
    assert_eq!(
        mark_answer.await.unwrap(),
        Response::CheckMarked { check: CheckId(1) }
    );
}

#[tokio::test]
async fn a_mark_sent_right_after_an_accepted_close_is_refused_at_once() {
    let mut h = Harness::new(1);
    let session = h.open(h.worktree("one")).await;
    h.request(mark_passed(&session)).await;
    h.controls.stop_hangs.store(true, Ordering::SeqCst);
    let (close_reply, close_answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: session.clone(),
                summary: "".into(),
            },
            close_reply,
            None,
        )
        .await;
    h.controls.started.notified().await;
    let (mark_reply, mark_answer) = oneshot::channel();
    h.registry
        .handle(mark_passed(&session), mark_reply, None)
        .await;
    assert!(refused(mark_answer.await.unwrap()).contains("closed"));
    h.controls.release.notify_one();
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    h.registry.finished(finished).await;
    assert!(matches!(close_answer.await.unwrap(), Response::Closed(_)));
}

#[tokio::test]
async fn an_unusable_videos_folder_refuses_close_and_keeps_the_session_open() {
    let mut h = Harness::new(1);
    let session = h.open(h.worktree("one")).await;
    h.request(mark_passed(&session)).await;
    let videos = h.directory.path().join("videos");
    std::fs::write(&videos, "not a folder").unwrap();
    let message = refused(
        h.request(Request::CloseSession {
            session: session.clone(),
            summary: "".into(),
        })
        .await,
    );
    assert!(message.contains("close_session"), "{message}");
    assert!(matches!(
        h.request(Request::CallTool {
            session: session.clone(),
            tool: "click".into(),
            arguments: json!({})
        })
        .await,
        Response::ToolResult(_)
    ));
    std::fs::remove_file(&videos).unwrap();
    assert!(matches!(h.finish(session).await, Response::Closed(_)));
}

#[tokio::test]
async fn an_empty_checklist_refuses_close_until_a_check_is_added() {
    let mut h = Harness::new(1);
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree: h.worktree("one"),
            goal: "goal".into(),
            checks: vec![],
        })
        .await;
    let Response::Opened {
        session, checks, ..
    } = response
    else {
        panic!("open refused");
    };
    assert!(checks.is_empty());
    let message = refused(
        h.request(Request::CloseSession {
            session: session.clone(),
            summary: "".into(),
        })
        .await,
    );
    assert!(message.contains("no Checks"), "{message}");
    assert_eq!(
        h.request(Request::AddCheck {
            session: session.clone(),
            text: "criterion".into()
        })
        .await,
        Response::CheckAdded { check: CheckId(1) }
    );
    assert!(matches!(h.finish(session).await, Response::Closed(_)));
}
