use super::*;

struct FailSessionFocus;
impl ProcessRunner for FailSessionFocus {
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
                Some("activewindow") => Ok(r#"{"address":"0xdef"}"#.into()),
                Some("eval")
                    if args.get(1).is_some_and(|arg| {
                        arg.contains("hl.dsp.focus") && arg.contains("address:0xabc")
                    }) =>
                {
                    Err(ProcessError::Failed {
                        program: "fake".into(),
                        message: "focus rejected".into(),
                    })
                }
                _ => Ok("ok".into()),
            }
        })
    }
}

fn controller(
    runner: Arc<dyn ProcessRunner>,
) -> (
    Control,
    mpsc::Receiver<Command>,
    Controller,
    watch::Receiver<SessionSnapshot>,
) {
    let (control, receiver) = Control::new();
    let (events, snapshots) = watch::channel(SessionSnapshot {
        control: ControlState::Verifier,
        state: crate::ipc::SessionState::Open,
        checks: vec![],
        last_action: None,
    });
    let controller = Controller {
        control: control.clone(),
        runner,
        compositor: Arc::new(FakeCompositor(StdMutex::new(None))),
        clock: Arc::new(crate::clock::MonotonicClock),
        events,
        opened: Instant::now(),
        window: Some(WindowAddress("0xabc".into())),
        project: "app".into(),
    };
    (control, receiver, controller, snapshots)
}

#[tokio::test]
async fn failed_clients_check_leaves_help_waiting() {
    let (control, receiver, controller, mut snapshots) = controller(Arc::new(BadClients));
    let task = tokio::spawn(controller.run(receiver, Box::pin(std::future::pending())));
    let (reply, mut response) = oneshot::channel();
    control
        .commands
        .send(Command::Help {
            arrival: control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    while !matches!(
        snapshots.borrow_and_update().control,
        ControlState::HelpRequest { .. }
    ) {
        snapshots.changed().await.unwrap();
    }
    control.commands.send(Command::TakeControl).await.unwrap();
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
    assert!(
        matches!(&snapshots.borrow().control, ControlState::HelpRequest { reason, .. } if reason == "2FA code")
    );
    assert!(response.try_recv().is_err());
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), control.gate.enter())
            .await
            .unwrap(),
        super::super::super::gate::Entry::Run(_)
    ));
    assert!(control.drain().await.is_empty());
    control.stop().await;
    task.await.unwrap();
}

#[tokio::test]
async fn failed_focus_after_host_change_records_one_span() {
    let (control, receiver, controller, mut snapshots) = controller(Arc::new(FailSessionFocus));
    let task = tokio::spawn(controller.run(receiver, Box::pin(std::future::pending())));
    control.commands.send(Command::TakeControl).await.unwrap();
    while !matches!(
        snapshots.borrow_and_update().control,
        ControlState::HandBack { .. }
    ) {
        snapshots.changed().await.unwrap();
    }
    assert_eq!(control.drain().await.len(), 1);
    control.stop().await;
    task.await.unwrap();
    assert!(control.drain().await.is_empty());
}

#[tokio::test]
async fn help_sets_orange_border_and_take_control_clears_it() {
    let (control, task, calls, mut snapshots) = fixture(false);
    let (reply, _response) = oneshot::channel();
    control
        .commands
        .send(Command::Help {
            arrival: control.gate.arrival(),
            reason: "2FA code".into(),
            reply,
        })
        .await
        .unwrap();
    while !matches!(
        snapshots.borrow_and_update().control,
        ControlState::HelpRequest { .. }
    ) {
        snapshots.changed().await.unwrap();
    }
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    control.stop().await;
    task.await.unwrap();
    let calls = calls.lock().unwrap();
    let borders: Vec<_> = calls
        .iter()
        .filter(|call| call.contains("inactive_border_color"))
        .collect();
    assert_eq!(borders.len(), 2, "{calls:?}");
    let orange = crate::viewer::theme::load().orange;
    assert!(borders[0].contains(&format!("rgb({})", orange.trim_start_matches('#'))));
    assert!(borders[1].contains("unset"));
}

#[tokio::test]
async fn repeated_take_and_done_without_take_do_not_create_extra_spans() {
    let (control, task, calls, mut snapshots) = fixture(false);
    control.commands.send(Command::Done).await.unwrap();
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    let after_first = Instant::now();
    control.commands.send(Command::TakeControl).await.unwrap();
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.contains("clients -j"))
            .count(),
        1
    );
    control.commands.send(Command::Done).await.unwrap();
    while !matches!(
        snapshots.borrow_and_update().control,
        ControlState::HandBack { .. }
    ) {
        snapshots.changed().await.unwrap();
    }
    control.commands.send(Command::Done).await.unwrap();
    control.stop().await;
    task.await.unwrap();
    let spans = control.drain().await;
    assert_eq!(spans.len(), 1);
    assert!(spans[0].start <= after_first);
}

struct FailNoFocus;
impl ProcessRunner for FailNoFocus {
    fn output<'a>(
        &'a self,
        _: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            if args
                .iter()
                .any(|arg| arg.contains("no_focus") && arg.contains("\"1\""))
            {
                return Err(ProcessError::Failed {
                    program: "fake".into(),
                    message: "rejected".into(),
                });
            }
            Ok(match args.first().copied() {
                Some("clients") => r#"[{"class":"editor","address":"0xdef","focusHistoryID":0}]"#,
                Some("activewindow") => r#"{"address":"0xdef"}"#,
                Some("binds") => "[]",
                _ => "ok",
            }
            .into())
        })
    }
}
#[tokio::test]
async fn ending_with_failed_no_focus_releases_gate_and_records_span() {
    let (control, receiver, controller, mut snapshots) = controller(Arc::new(FailNoFocus));
    let (end, ended) = oneshot::channel();
    let task = tokio::spawn(controller.run(
        receiver,
        Box::pin(async {
            let _ = ended.await;
        }),
    ));
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    let held = tokio::spawn({
        let gate = control.gate.clone();
        async move { gate.enter().await }
    });
    tokio::task::yield_now().await;
    end.send(()).unwrap();
    task.await.unwrap();
    let entry = tokio::time::timeout(std::time::Duration::from_millis(100), held).await;
    assert!(matches!(
        entry,
        Ok(Ok(super::super::super::gate::Entry::HandBack(_)))
    ));
    assert_eq!(control.drain().await.len(), 1);
}
