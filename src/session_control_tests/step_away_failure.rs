use super::*;
use crate::{
    ipc::TakeoverPhase,
    session::gate::{Arrival, Entry},
};

const CLEAR_BORDER: &str = "prop = \"inactive_border_color\", value = \"unset\"";
const ENTER_SUBMAP: &str = "submap(\"keek\")";

struct HostRunner {
    calls: StdMutex<Vec<String>>,
    fail_on: StdMutex<Option<&'static str>>,
    active_window: &'static str,
}

impl ProcessRunner for HostRunner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            let call = format!("{program} {}", args.join(" "));
            let mut fail_on = self.fail_on.lock().unwrap();
            let fails = fail_on.is_some_and(|needle| call.contains(needle));
            self.calls.lock().unwrap().push(call);
            if fails {
                fail_on.take();
                return Err(ProcessError::Failed {
                    program: program.into(),
                    message: "host command rejected".into(),
                });
            }
            Ok(match args.first().copied() {
                Some("clients") => r#"[{"class":"editor","address":"0xdef","focusHistoryID":1},{"class":"aquamarine","address":"0xabc","focusHistoryID":0}]"#,
                Some("activewindow") => self.active_window,
                Some("binds") => r#"[{"submap":"keek","key":"Escape","modmask":64},{"submap":"keek","key":"Tab","modmask":64}]"#,
                _ => "ok",
            }
            .into())
        })
    }
}

fn host(
    active_window: &'static str,
) -> (
    Control,
    tokio::task::JoinHandle<()>,
    Arc<HostRunner>,
    watch::Receiver<SessionSnapshot>,
) {
    let (control, receiver) = Control::new();
    let (events, snapshots) = watch::channel(SessionSnapshot {
        control: ControlState::Verifier,
        state: crate::ipc::SessionState::Open,
        checks: vec![],
        reminded: false,
        last_action: None,
    });
    let runner = Arc::new(HostRunner {
        calls: StdMutex::new(Vec::new()),
        fail_on: StdMutex::new(None),
        active_window,
    });
    let controller = Controller {
        control: control.clone(),
        runner: runner.clone(),
        compositor: Arc::new(FakeCompositor(StdMutex::new(None))),
        clock: Arc::new(crate::clock::MonotonicClock),
        events,
        opened: Instant::now(),
        window: Some(WindowAddress("0xabc".into())),
        project: "app".into(),
        clipboard: Arc::new(NoClipboard),
    };
    (
        control,
        tokio::spawn(controller.run(receiver, Box::pin(std::future::pending()))),
        runner,
        snapshots,
    )
}

async fn apply(control: &Control, command: Command) {
    control.commands.send(command).await.unwrap();
    // Withdraw acknowledges after the preceding command without affecting a Takeover.
    let (reply, applied) = oneshot::channel();
    control
        .commands
        .send(Command::Withdraw(reply))
        .await
        .unwrap();
    applied.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn done_from_a_pause_hands_back_even_when_clearing_the_border_fails() {
    let (control, task, runner, snapshots) = host(r#"{"address":"0xdef"}"#);
    apply(&control, Command::TakeControl).await;
    let call = tokio::spawn({
        let gate = control.gate.clone();
        async move { gate.enter().await }
    });
    apply(&control, Command::StepAway).await;
    assert!(matches!(
        snapshots.borrow().control,
        ControlState::Takeover {
            phase: TakeoverPhase::Paused { .. },
            ..
        }
    ));
    assert!(!call.is_finished(), "Step away released the held call");
    *runner.fail_on.lock().unwrap() = Some(CLEAR_BORDER);

    apply(&control, Command::Done).await;

    assert!(runner.fail_on.lock().unwrap().is_none());
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), call)
        .await
        .expect("Done left the Verifier's call held after the border failed")
        .unwrap();
    let Entry::HandBack(span) = result else {
        panic!("Done did not return HandBack");
    };
    assert_eq!(span.paused.len(), 1, "{span:?}");
    assert!(span.start <= span.paused[0].start && span.paused[0].end == span.end);
    control.stop().await;
    task.await.unwrap();
}

#[tokio::test]
async fn step_away_restores_running_control_when_focus_stays_on_the_session_window() {
    let (control, task, runner, mut snapshots) = host(r#"{"address":"0xabc"}"#);
    apply(&control, Command::TakeControl).await;
    let running = snapshots.borrow_and_update().control.clone();
    let before = runner.calls.lock().unwrap().len();

    apply(&control, Command::StepAway).await;

    assert!(
        snapshots.has_changed().unwrap(),
        "expected a fresh snapshot"
    );
    assert_eq!(snapshots.borrow().control, running);
    assert!(matches!(
        running,
        ControlState::Takeover {
            phase: TakeoverPhase::Running,
            shortcuts: HostShortcuts::Captured,
            ..
        }
    ));
    let calls = runner.calls.lock().unwrap()[before..].to_vec();
    let last_no_focus = calls
        .iter()
        .rfind(|call| call.contains("prop = \"no_focus\""));
    assert!(
        last_no_focus.is_some_and(|call| call.contains("value = \"0\"")),
        "{calls:?}"
    );
    let last_focus = calls.iter().rfind(|call| call.contains("hl.dsp.focus"));
    assert!(
        last_focus.is_some_and(|call| call.contains("address:0xabc")),
        "{calls:?}"
    );
    assert!(
        calls.iter().any(|call| call.contains(ENTER_SUBMAP)),
        "{calls:?}"
    );
    assert!(
        calls.iter().all(|call| !call.contains("Takeover paused")),
        "{calls:?}"
    );
    assert!(matches!(control.gate.arrival(), Arrival::DuringTakeover(_)));
    control.stop().await;
    task.await.unwrap();
}

#[tokio::test]
async fn resume_restores_paused_control_when_entering_the_submap_fails() {
    let (control, task, runner, snapshots) = host(r#"{"address":"0xdef"}"#);
    apply(&control, Command::TakeControl).await;
    apply(&control, Command::StepAway).await;
    let paused = snapshots.borrow().control.clone();
    assert!(matches!(
        paused,
        ControlState::Takeover {
            phase: TakeoverPhase::Paused { .. },
            ..
        }
    ));
    let before = runner.calls.lock().unwrap().len();
    *runner.fail_on.lock().unwrap() = Some(ENTER_SUBMAP);

    apply(&control, Command::Resume).await;

    assert!(runner.fail_on.lock().unwrap().is_none());
    assert_eq!(snapshots.borrow().control, paused);
    let calls = runner.calls.lock().unwrap()[before..].to_vec();
    let last_focus = calls.iter().rfind(|call| call.contains("hl.dsp.focus"));
    assert!(
        last_focus.is_some_and(|call| call.contains("address:0xdef")),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .all(|call| !call.starts_with("omarchy-notification-dismiss ")),
        "{calls:?}"
    );
    assert!(matches!(control.gate.arrival(), Arrival::DuringTakeover(_)));
    control.stop().await;
    task.await.unwrap();
}
