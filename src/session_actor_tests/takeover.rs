use super::*;
#[tokio::test(start_paused = true)]
async fn a_forwarded_call_during_takeover_returns_a_note_and_fresh_screenshot_without_acting() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "takeover", false).await;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let held = tokio::spawn({
        let sender = actor.sender.clone();
        let gate = actor.control.gate.clone();
        async move { forward(&sender, &gate, "click").await }
    });
    tokio::task::yield_now().await;
    assert!(!held.is_finished());
    actor.clock.elapsed.fetch_add(42000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let Response::ToolResult(value) = held.await.unwrap() else {
        panic!("expected tool result")
    };
    assert_eq!(
        value["content"][0]["text"],
        "The developer controlled the session for 0:42. The screen may have changed."
    );
    assert!(
        actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool != "click")
    );
    assert!(
        actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "get_desktop_state")
    );
    drop(actor.sender);
    actor.task.await.unwrap();
    let timeline: SessionTimeline =
        serde_json::from_slice(&std::fs::read(actor.folder.root().join("timeline.json")).unwrap())
            .unwrap();
    assert_eq!(
        timeline
            .events()
            .iter()
            .filter(|event| matches!(event, TimelineEvent::Takeover { .. }))
            .count(),
        1
    );
    assert!(
        timeline.events().iter().all(
            |event| !matches!(event,TimelineEvent::Call{tool,..} if tool=="get_desktop_state")
        )
    );
}
#[tokio::test(start_paused = true)]
async fn a_help_request_notifies_once_until_take_control_and_waits_for_done_without_a_timeout() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "help", false).await;
    let (reply, mut answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HelpRequest { .. }
    ) {
        events.changed().await.unwrap();
    }
    tokio::time::advance(Duration::from_secs(7200)).await;
    assert!(answer.try_recv().is_err());
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor.clock.elapsed.fetch_add(42000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let Response::ToolResult(value) = answer.await.unwrap() else {
        panic!("expected tool result")
    };
    assert_eq!(
        value["content"][0]["text"],
        "The developer controlled the session for 0:42 (2FA code). The screen may have changed."
    );
    let calls = actor.calls.lock().unwrap().clone();
    let notifications: Vec<_> = calls
        .iter()
        .filter(|(tool, _)| tool == "omarchy-notification-send")
        .collect();
    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].1.to_string().contains("2FA code · app"));
    assert!(
        notifications[0]
            .1
            .to_string()
            .contains(r#""-u","critical","Agent needs you""#)
    );
    let dismissed: Vec<_> = calls
        .iter()
        .filter(|(tool, _)| tool == "omarchy-notification-dismiss")
        .collect();
    assert_eq!(dismissed.len(), 1);
    assert_eq!(dismissed[0].1, json!(["Agent needs you"]));
    drop(actor.sender);
    actor.task.await.unwrap();
}

pub(super) struct Runner {
    pub inner: EvidenceRunner,
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    pub fail_no_focus: Arc<AtomicBool>,
}
impl crate::process::ProcessRunner for Runner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            if program == "hyprctl"
                || program == "omarchy-notification-send"
                || program == "omarchy-notification-dismiss"
            {
                self.calls
                    .lock()
                    .unwrap()
                    .push((program.into(), json!(args)));
                if self.fail_no_focus.load(Ordering::SeqCst)
                    && args
                        .iter()
                        .any(|arg| arg.contains("no_focus") && arg.contains("\"1\""))
                {
                    return Err(crate::process::ProcessError::Failed {
                        program: "fake".into(),
                        message: "no_focus rejected".into(),
                    });
                }
                return Ok(match args.first().copied() {
                    Some("clients") => {
                        r#"[{"class":"editor","address":"0xdef","focusHistoryID":0}]"#
                    }
                    Some("binds") => "[]",
                    Some("activewindow") => r#"{"address":"0xdef"}"#,
                    _ => "ok",
                }
                .into());
            }
            self.inner.output(program, args, cwd).await
        })
    }
}
#[tokio::test(start_paused = true)]
async fn a_call_in_flight_finishes_before_takeover_changes_host_focus() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "inflight", false).await;
    let call_task = tokio::spawn({
        let sender = actor.sender.clone();
        let gate = actor.control.gate.clone();
        async move { forward(&sender, &gate, "slow").await }
    });
    actor.entered.notified().await;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert!(
        actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool != "hyprctl")
    );
    actor.release.notify_one();
    call_task.await.unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let calls = actor.calls.lock().unwrap().clone();
    let ended = calls
        .iter()
        .position(|(tool, _)| tool == "driver finished")
        .unwrap();
    let focused = calls
        .iter()
        .position(|(tool, args)| tool == "hyprctl" && args.to_string().contains("hl.dsp.focus"))
        .unwrap();
    assert!(ended < focused, "{calls:?}");
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    drop(actor.sender);
    actor.task.await.unwrap();
}

pub(super) async fn forward(
    sender: &mpsc::Sender<ActorRequest>,
    gate: &gate::Gate,
    tool: &str,
) -> Response {
    let (reply, response) = oneshot::channel();
    sender
        .send(ActorRequest::Call {
            arrival: gate.arrival(),
            tool: tool.into(),
            arguments: json!({"session":"keek-name"}),
            reply,
        })
        .await
        .unwrap();
    response.await.unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_held_close_runs_after_done() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "held-close", false).await;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let mut reply = close(&actor).await;
    tokio::task::yield_now().await;
    assert!(reply.try_recv().is_err());
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let finished = actor.task.await.unwrap();
    assert!(actor.closed.load(Ordering::SeqCst));
    assert!(finished.reply.is_some());
}
#[tokio::test(start_paused = true)]
async fn a_help_request_during_takeover_returns_at_that_done() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "current-help", false).await;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "already helping".into(),
            reply,
        })
        .await
        .unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    assert!(matches!(answer.await.unwrap(), Response::ToolResult(_)));
    assert_eq!(
        actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "omarchy-notification-send")
            .count(),
        0
    );
    drop(actor.sender);
    actor.task.await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn fresh_hand_back_content_is_masked_and_follows_the_note() {
    let root = tempfile::tempdir().unwrap();
    let credentials =
        crate::credentials::Credentials::parse(Path::new("fixture.env"), "PASSWORD=made-up-value")
            .unwrap();
    let (actor, _) =
        start_actor_with_credentials(root.path(), "masked-help", false, None, credentials).await;
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HelpRequest { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let Response::ToolResult(value) = answer.await.unwrap() else {
        panic!("expected screenshot")
    };
    assert!(
        value["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("The developer")
    );
    assert_eq!(value["content"][1]["text"], "[masked]");
    assert_eq!(value["content"][2]["type"], "image");
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn calls_queued_behind_a_help_request_return_hand_back_without_acting() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "queued-help", false).await;
    let (reply, help) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HelpRequest { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let (reply, queued) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "click".into(),
            arguments: json!({"session":"keek-name"}),
            reply,
        })
        .await
        .unwrap();
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    assert!(matches!(help.await.unwrap(), Response::ToolResult(_)));
    let Response::ToolResult(value) = queued.await.unwrap() else {
        panic!("expected hand-back")
    };
    assert!(
        value["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("The developer")),
        "{value}"
    );
    assert!(
        actor
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool != "click")
    );
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_help_request_is_refused_when_the_sandbox_ends_before_take_control() {
    let root = tempfile::tempdir().unwrap();
    let mut actor = start_actor(root.path(), "ended-help", false).await;
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HelpRequest { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor.ended.send_replace(());
    let response = tokio::time::timeout(Duration::from_secs(1), answer).await;
    if response.is_err() {
        actor.task.abort();
    }
    let _ = (&mut actor.task).await;
    assert!(
        matches!(response, Ok(Ok(Response::Refused { .. }))),
        "{response:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_fresh_screenshot_finishes_before_the_next_takeover_changes_focus() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "screenshot-permit", false).await;
    *actor.screenshot.lock().unwrap() = fake_driver::Screenshot::Held;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "click".into(),
            arguments: json!({"session":"keek-name"}),
            reply,
        })
        .await
        .unwrap();
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    actor.entered.notified().await;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    let focused = actor
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, args)| {
            tool == "hyprctl"
                && args.to_string().contains("hl.dsp.focus")
                && args.to_string().contains("address:0xabc")
        })
        .count();
    actor.release.notify_one();
    answer.await.unwrap();
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    drop(actor.sender);
    actor.task.await.unwrap();
    assert_eq!(
        focused, 1,
        "a second Takeover focused the desktop before the screenshot finished"
    );
}

#[tokio::test(start_paused = true)]
async fn screenshot_failure_returns_note_and_error() {
    screenshot_error(fake_driver::Screenshot::Failed).await;
}
#[tokio::test(start_paused = true)]
async fn screenshot_tool_error_returns_note_and_error() {
    screenshot_error(fake_driver::Screenshot::ReportedFailure).await;
}
#[tokio::test(start_paused = true)]
async fn screenshot_timeout_returns_note_and_error() {
    screenshot_error(fake_driver::Screenshot::Held).await;
}
async fn screenshot_error(mode: fake_driver::Screenshot) {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "screenshot-error", false).await;
    *actor.screenshot.lock().unwrap() = mode;
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "click".into(),
            arguments: json!({"session":"keek-name"}),
            reply,
        })
        .await
        .unwrap();
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(151), answer).await;
    let Ok(Ok(Response::ToolResult(value))) = result else {
        actor.task.abort();
        panic!("expected bounded tool result: {result:?}")
    };
    assert!(
        value["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("The developer")
    );
    assert_eq!(
        value["content"][1]["text"],
        "keek couldn't take a fresh screenshot. Take one before you act."
    );
    drop(actor.sender);
    actor.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn closing_after_a_takeover_files_it_in_the_report() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "report-takeover", false).await;
    actor.clock.elapsed.fetch_add(5000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor.clock.elapsed.fetch_add(42000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HandBack { .. }
    ) {
        events.changed().await.unwrap();
    }
    let Response::Closed(value) = finish(actor).await else {
        panic!("expected filed evidence")
    };
    let report = std::fs::read_to_string(value["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(report.contains("## Takeovers"));
    assert!(
        report.contains("the developer stepped in and was in control for 0:42."),
        "{report}"
    );
}

#[tokio::test]
async fn closing_after_an_answered_help_request_files_the_wait_in_the_report() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "report-help", false).await;
    actor.clock.elapsed.fetch_add(5000, Ordering::SeqCst);
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::RequestHelp {
            arrival: actor.control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::HelpRequest { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor.clock.elapsed.fetch_add(283_000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    actor.clock.elapsed.fetch_add(22_000, Ordering::SeqCst);
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    answer.await.unwrap();

    let Response::Closed(value) = finish(actor).await else {
        panic!("expected filed evidence")
    };
    let report = std::fs::read_to_string(value["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(
        report.contains(
            "the developer answered a Help Request after 4:43 and was in control for 0:22: 2FA code."
        ),
        "{report}"
    );
}
