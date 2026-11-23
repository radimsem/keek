#[path = "session_control_tests/failure.rs"]
mod failure;
#[path = "session_control_tests/focus_restore.rs"]
mod focus_restore;
#[path = "session_control_tests/hand_back.rs"]
mod hand_back;
#[path = "session_control_tests/paste.rs"]
mod paste;
#[path = "session_control_tests/step_away.rs"]
mod step_away;
#[path = "session_control_tests/step_away_failure.rs"]
mod step_away_failure;
#[path = "session_control_tests/unanswered.rs"]
mod unanswered;
use super::*;
use crate::compositor::{
    CompositorFuture, EventsFuture, Placement, SlotFuture, WindowEvents, WindowFuture, WindowPid,
};
use std::sync::Mutex as StdMutex;
struct NoClipboard;
impl crate::takeover_clipboard::NestedClipboard for NoClipboard {
    fn copy(
        &self,
        _: crate::takeover_clipboard::CarriedText,
    ) -> crate::takeover_clipboard::ClipboardFuture<'_> {
        Box::pin(async { Ok(()) })
    }
    fn clear(&self) -> crate::takeover_clipboard::ClipboardFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}
struct DelayedConfirmation {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
impl ProcessRunner for DelayedConfirmation {
    fn output<'a>(
        &'a self,
        _: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            match args.first().copied() {
                Some("clients") => {
                    Ok(r#"[{"class":"editor","address":"0xdef","focusHistoryID":0}]"#.into())
                }
                Some("binds") => Ok("[]".into()),
                Some("activewindow") => {
                    self.entered.notify_one();
                    self.release.notified().await;
                    Ok(r#"{"address":"0xdef"}"#.into())
                }
                _ => Ok("ok".into()),
            }
        })
    }
}
#[tokio::test]
async fn help_arriving_during_done_does_not_leave_a_stale_request() {
    let (control, receiver) = Control::new();
    let (events, mut snapshots) = watch::channel(SessionSnapshot {
        control: ControlState::Verifier,
        state: crate::ipc::SessionState::Open,
        checks: vec![],
        reminded: false,
        last_action: None,
    });
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let controller = Controller {
        control: control.clone(),
        runner: Arc::new(DelayedConfirmation {
            entered: entered.clone(),
            release: release.clone(),
        }),
        compositor: Arc::new(FakeCompositor(StdMutex::new(None))),
        clock: Arc::new(crate::clock::MonotonicClock),
        events,
        opened: Instant::now(),
        window: Some(WindowAddress("0xabc".into())),
        project: "app".into(),
        clipboard: Arc::new(NoClipboard),
    };
    let task = tokio::spawn(controller.run(receiver, Box::pin(std::future::pending())));
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    let (reply, waiting) = oneshot::channel();
    control.commands.send(Command::Done).await.unwrap();
    entered.notified().await;
    control
        .commands
        .send(Command::Help {
            arrival: control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        snapshots.borrow().control,
        ControlState::HandBack { .. }
    ));
    control.stop().await;
    task.await.unwrap();
}
struct BadClients;
impl ProcessRunner for BadClients {
    fn output<'a>(
        &'a self,
        _: &'a str,
        _: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async {
            Err(ProcessError::Failed {
                program: "fake".into(),
                message: "clients failed".into(),
            })
        })
    }
}
#[tokio::test(start_paused = true)]
async fn failure_before_host_changes_reopens_the_gate_without_a_span() {
    let (control, _) = Control::new();
    let (events, _) = watch::channel(SessionSnapshot {
        control: ControlState::Verifier,
        state: crate::ipc::SessionState::Open,
        checks: vec![],
        reminded: false,
        last_action: None,
    });
    let controller = Controller {
        control: control.clone(),
        runner: Arc::new(BadClients),
        compositor: Arc::new(FakeCompositor(StdMutex::new(None))),
        clock: Arc::new(crate::clock::MonotonicClock),
        events,
        opened: Instant::now(),
        window: Some(WindowAddress("0xabc".into())),
        project: "app".into(),
        clipboard: Arc::new(NoClipboard),
    };
    assert!(controller.take_control(None).await.is_err());
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), control.gate.enter()).await,
        Ok(super::super::gate::Entry::Run(_))
    ));
    assert!(control.drain().await.is_empty());
}
struct FakeCompositor(StdMutex<Option<mpsc::Receiver<crate::hypr::HyprEvent>>>);
struct Events(mpsc::Receiver<crate::hypr::HyprEvent>);
impl crate::compositor::TakeoverEvents for Events {
    fn next(&mut self) -> crate::compositor::EventFuture<'_> {
        Box::pin(async { Ok(self.0.recv().await) })
    }
}
impl Compositor for FakeCompositor {
    fn takeover_events(&self) -> crate::compositor::TakeoverEventsFuture<'_> {
        let receiver = self.0.lock().unwrap().take();
        Box::pin(async move {
            Ok(receiver
                .map(|rx| Box::new(Events(rx)) as Box<dyn crate::compositor::TakeoverEvents>))
        })
    }
    fn place_window(
        &self,
        _: Option<WindowPid>,
        _: Placement,
        _: Option<WindowEvents>,
    ) -> WindowFuture<'_> {
        Box::pin(async { Ok(WindowAddress("0xabc".into())) })
    }
    fn slot(
        &self,
        _: crate::viewer::SlotId,
        _: usize,
        _: Option<crate::project::WindowSize>,
    ) -> SlotFuture<'_> {
        Box::pin(async {
            Err(ProcessError::Failed {
                program: "fake".into(),
                message: "unused".into(),
            })
        })
    }
    fn window_events(&self) -> EventsFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn install(&self) -> CompositorFuture<'_> {
        Box::pin(async { Ok(()) })
    }
    fn remove(&self) -> CompositorFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}
struct Runner {
    calls: Arc<StdMutex<Vec<String>>>,
    reset_fails: bool,
    eval_output: &'static str,
}
impl ProcessRunner for Runner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            let command = format!("{program} {}", args.join(" "));
            self.calls.lock().unwrap().push(command.clone());
            if args.first() == Some(&"clients") {
                return Ok(r#"[{"class":"editor","address":"0xdef","focusHistoryID":1},{"class":"aquamarine","address":"0xabc","focusHistoryID":0}]"#.into());
            }
            if args.first() == Some(&"activewindow") {
                return Ok(r#"{"address":"0xdef"}"#.into());
            }
            if args.first() == Some(&"binds") {
                return Ok(r#"[{"submap":"keek","key":"Escape","modmask":64}]"#.into());
            }
            if self.reset_fails && command.contains("submap(\"reset\")") {
                return Err(ProcessError::Failed {
                    program: program.into(),
                    message: "reset rejected".into(),
                });
            }
            Ok(self.eval_output.into())
        })
    }
}
type Fixture = (
    Control,
    tokio::task::JoinHandle<()>,
    Arc<StdMutex<Vec<String>>>,
    watch::Receiver<SessionSnapshot>,
);
fn fixture(reset_fails: bool) -> Fixture {
    fixture_with_signals(reset_fails, None, Box::pin(std::future::pending()))
}
fn fixture_with_signals(
    reset_fails: bool,
    host_events: Option<mpsc::Receiver<crate::hypr::HyprEvent>>,
    ended: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
) -> Fixture {
    fixture_with_output(reset_fails, host_events, ended, "ok")
}
fn fixture_with_output(
    reset_fails: bool,
    host_events: Option<mpsc::Receiver<crate::hypr::HyprEvent>>,
    ended: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    eval_output: &'static str,
) -> Fixture {
    let (control, receiver) = Control::new();
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let (events, snapshots) = watch::channel(SessionSnapshot {
        control: ControlState::Verifier,
        state: crate::ipc::SessionState::Open,
        checks: vec![],
        reminded: false,
        last_action: None,
    });
    let controller = Controller {
        control: control.clone(),
        runner: Arc::new(Runner {
            calls: calls.clone(),
            reset_fails,
            eval_output,
        }),
        compositor: Arc::new(FakeCompositor(StdMutex::new(host_events))),
        clock: Arc::new(crate::clock::MonotonicClock),
        events,
        opened: Instant::now(),
        window: Some(WindowAddress("0xabc".into())),
        project: "app".into(),
        clipboard: Arc::new(NoClipboard),
    };
    (
        control,
        tokio::spawn(controller.run(receiver, ended)),
        calls,
        snapshots,
    )
}
async fn taking(snapshots: &mut watch::Receiver<SessionSnapshot>) {
    loop {
        if matches!(
            snapshots.borrow_and_update().control,
            ControlState::Takeover { .. }
        ) {
            break;
        }
        snapshots.changed().await.unwrap();
    }
}
#[tokio::test]
async fn done_restores_no_focus_and_previous_window_even_when_reset_fails() {
    let (control, task, calls, mut snapshots) = fixture(true);
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    control.stop().await;
    task.await.unwrap();
    let calls = calls.lock().unwrap();
    let reset = calls
        .iter()
        .position(|call| call.contains("submap(\"reset\")"))
        .unwrap();
    assert!(
        calls[reset + 1].contains("no_focus") && calls[reset + 1].contains("\"1\""),
        "{calls:?}"
    );
    assert!(calls[reset + 2].contains("clients -j"), "{calls:?}");
    assert!(calls[reset + 3].contains("address:0xdef"), "{calls:?}");
}
#[tokio::test]
async fn takeover_waits_for_the_call_before_any_host_operation_and_records_without_a_held_call() {
    let (control, task, calls, mut snapshots) = fixture(false);
    let super::super::gate::Entry::Run(permit) = control.gate.enter().await else {
        panic!("expected permit")
    };
    calls.lock().unwrap().push("driver started".into());
    control.commands.send(Command::TakeControl).await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(calls.lock().unwrap().as_slice(), ["driver started"]);
    calls.lock().unwrap().push("driver finished".into());
    drop(permit);
    taking(&mut snapshots).await;
    control.stop().await;
    task.await.unwrap();
    {
        let calls = calls.lock().unwrap();
        assert!(calls[2].starts_with("hyprctl clients"), "{calls:?}");
    }
    let spans = control.drain().await;
    assert_eq!(spans.len(), 1);
    assert!(spans[0].reason.is_none());
}
#[test]
fn snippet_detection_requires_the_keek_escape_super_binding() {
    for (submap, key, mask, expected) in [
        ("keek", "Escape", 64, true),
        ("keek", "escape", 65, true),
        ("other", "Escape", 64, false),
        ("keek", "Escape", 0, false),
        ("keek", "A", 64, false),
    ] {
        assert_eq!(
            has_submap(&serde_json::json!([{"submap":submap,"key":key,"modmask":mask}])),
            expected
        );
    }
}

#[tokio::test]
async fn leaving_the_keek_submap_finishes_the_takeover_without_polling() {
    let (sender, receiver) = mpsc::channel(4);
    let (control, task, _, mut snapshots) =
        fixture_with_signals(false, Some(receiver), Box::pin(std::future::pending()));
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    sender
        .send(crate::hypr::HyprEvent::Submap("keek".into()))
        .await
        .unwrap();
    sender
        .send(crate::hypr::HyprEvent::Submap(String::new()))
        .await
        .unwrap();
    loop {
        if matches!(
            snapshots.borrow_and_update().control,
            ControlState::HandBack { .. }
        ) {
            break;
        }
        snapshots.changed().await.unwrap();
    }
    control.stop().await;
    task.await.unwrap();
    assert_eq!(control.drain().await.len(), 1);
}
#[tokio::test]
async fn a_sandbox_failure_restores_the_host_before_the_controller_ends() {
    let (sender, receiver) = oneshot::channel();
    let (control, task, calls, mut snapshots) = fixture_with_signals(
        false,
        None,
        Box::pin(async {
            let _ = receiver.await;
        }),
    );
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    sender.send(()).unwrap();
    task.await.unwrap();
    let calls = calls.lock().unwrap();
    let reset = calls
        .iter()
        .position(|call| call.contains("submap(\"reset\")"))
        .unwrap();
    assert!(calls[reset + 1].contains("no_focus"));
    assert!(calls[reset + 2].contains("clients -j"));
    assert!(calls[reset + 3].contains("address:0xdef"));
    assert!(calls[reset + 4].contains("activewindow -j"));
}

#[tokio::test]
async fn rejected_eval_does_not_publish_a_takeover_or_reset_a_submap_it_did_not_enter() {
    let (control, task, calls, snapshots) = fixture_with_output(
        false,
        None,
        Box::pin(std::future::pending()),
        "property rejected",
    );
    control.commands.send(Command::TakeControl).await.unwrap();
    control.stop().await;
    task.await.unwrap();
    assert!(!matches!(
        snapshots.borrow().control,
        ControlState::Takeover { .. }
    ));
    let calls = calls.lock().unwrap();
    assert!(
        calls.iter().all(|call| !call.contains("submap(\"reset\")")),
        "{calls:?}"
    );
}
