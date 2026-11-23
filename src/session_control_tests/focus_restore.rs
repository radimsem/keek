use super::*;

struct FocusCase {
    clients: &'static str,
    active: &'static str,
    calls: Arc<StdMutex<Vec<String>>>,
}
impl ProcessRunner for FocusCase {
    fn output<'a>(
        &'a self,
        _: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(args.join(" "));
            Ok(match args.first().copied() {
                Some("clients") => self.clients,
                Some("activewindow") => self.active,
                _ => "ok",
            }
            .into())
        })
    }
}
fn focus_case(
    clients: &'static str,
    active: &'static str,
) -> (Controller, Arc<StdMutex<Vec<String>>>) {
    let (control, _) = Control::new();
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let (events, _) = watch::channel(SessionSnapshot {
        control: ControlState::Takeover {
            start_ms: Millis(0),
            shortcuts: HostShortcuts::Live,
            phase: TakeoverPhase::Running,
        },
        state: crate::ipc::SessionState::Open,
        checks: vec![],
        reminded: false,
        last_action: None,
    });
    (
        Controller {
            control,
            runner: Arc::new(FocusCase {
                clients,
                active,
                calls: calls.clone(),
            }),
            compositor: Arc::new(FakeCompositor(StdMutex::new(None))),
            clock: Arc::new(crate::clock::MonotonicClock),
            events,
            opened: Instant::now(),
            window: Some(WindowAddress("0xabc".into())),
            project: "app".into(),
            session: "app-12-00-00".parse().unwrap(),
            clipboard: Arc::new(NoClipboard),
        },
        calls,
    )
}
#[tokio::test]
async fn done_focuses_the_next_client_when_the_previous_window_is_gone() {
    let (controller, calls) = focus_case(
        r#"[{"class":"editor","address":"0x456","focusHistoryID":5},{"class":"editor","address":"0x123","focusHistoryID":1}]"#,
        r#"{"address":"0x123"}"#,
    );
    drop(controller.control.gate.close().await);
    let mut taking = Taking {
        start: Instant::now(),
        reason: None,
        previous: Some(WindowAddress("0xdef".into())),
        shortcuts: HostShortcuts::Live,
        phase: TakingPhase::Running,
        paused: Vec::new(),
    };
    assert!(controller.done(&mut taking).await.is_ok());
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call.contains("address:0x123"))
    );
}
#[tokio::test]
async fn failed_focus_confirmation_keeps_the_takeover_and_gate_closed() {
    let (controller, _) = focus_case(
        r#"[{"class":"editor","address":"0xdef","focusHistoryID":1}]"#,
        r#"{"address":"0xabc"}"#,
    );
    drop(controller.control.gate.close().await);
    controller.events.send_modify(|snapshot| {
        snapshot.control = ControlState::Takeover {
            start_ms: Millis(0),
            shortcuts: HostShortcuts::Captured,
            phase: TakeoverPhase::Running,
        }
    });
    let mut taking = Taking {
        start: Instant::now(),
        reason: None,
        previous: Some(WindowAddress("0xdef".into())),
        shortcuts: HostShortcuts::Captured,
        phase: TakingPhase::Running,
        paused: Vec::new(),
    };
    assert!(controller.done(&mut taking).await.is_err());
    assert!(matches!(
        controller.events.borrow().control,
        ControlState::Takeover {
            shortcuts: HostShortcuts::Live,
            ..
        }
    ));
    assert!(controller.control.drain().await.is_empty());
    let held = tokio::spawn({
        let gate = controller.control.gate.clone();
        async move { gate.enter().await }
    });
    tokio::task::yield_now().await;
    assert!(!held.is_finished());
    held.abort();
}
struct NoOtherClient;
impl ProcessRunner for NoOtherClient {
    fn output<'a>(
        &'a self,
        _: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            Ok(if args.first() == Some(&"clients") {
                r#"[{"class":"aquamarine","address":"0xabc","focusHistoryID":0}]"#
            } else {
                "ok"
            }
            .into())
        })
    }
}
#[tokio::test]
async fn done_without_another_client_keeps_the_gate_closed() {
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
        runner: Arc::new(NoOtherClient),
        compositor: Arc::new(FakeCompositor(StdMutex::new(None))),
        clock: Arc::new(crate::clock::MonotonicClock),
        events,
        opened: Instant::now(),
        window: Some(WindowAddress("0xabc".into())),
        project: "app".into(),
        session: "app-12-00-00".parse().unwrap(),
        clipboard: Arc::new(NoClipboard),
    };
    drop(control.gate.close().await);
    let mut taking = Taking {
        start: Instant::now(),
        reason: None,
        previous: None,
        shortcuts: HostShortcuts::Live,
        phase: TakingPhase::Running,
        paused: Vec::new(),
    };
    assert!(controller.done(&mut taking).await.is_err());
    let held = tokio::spawn({
        let gate = control.gate.clone();
        async move { gate.enter().await }
    });
    tokio::task::yield_now().await;
    assert!(!held.is_finished());
    held.abort();
    assert!(control.drain().await.is_empty());
}
