use super::*;
use std::time::Duration;

async fn open_with_progress(h: &mut Harness, worktree: PathBuf) -> (Response, Vec<String>) {
    let (reply, mut answer) = oneshot::channel();
    let (events, mut progress) = mpsc::channel(16);
    h.registry
        .handle(
            Request::OpenSession {
                project: "app".into(),
                worktree,
                goal: "goal".into(),
                checks: vec!["check".into()],
            },
            reply,
            Some(events),
        )
        .await;
    let response = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                response = &mut answer => return response.unwrap(),
                Some(prepared) = h.registry.preparing.join_next() => h.registry.prepared(prepared.unwrap()).await,
                Some(finished) = h.registry.tasks.join_next() => h.registry.finished(finished.unwrap()).await,
            }
        }
    })
    .await
    .expect("the open answers");
    let mut messages = Vec::new();
    while let Ok(Event::Progress { message }) = progress.try_recv() {
        messages.push(message);
    }
    (response, messages)
}

#[tokio::test]
async fn opening_a_session_reports_each_phase_in_order() {
    let mut h = Harness::new(2);
    let worktree = h.worktree("one");
    let (response, messages) = open_with_progress(&mut h, worktree).await;
    assert!(matches!(response, Response::Opened { .. }), "{response:?}");
    assert_eq!(
        messages,
        [
            "Building Project Layer",
            "Starting desktop",
            "Starting App Under Test"
        ]
    );
}

#[tokio::test]
async fn an_open_that_fails_at_the_desktop_never_reports_the_app_starting() {
    let mut h = Harness::new(2);
    h.controls
        .fail_open
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let worktree = h.worktree("one");
    let (response, messages) = open_with_progress(&mut h, worktree).await;
    assert!(matches!(response, Response::Refused { .. }), "{response:?}");
    assert_eq!(messages, ["Building Project Layer", "Starting desktop"]);
}
