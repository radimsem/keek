use super::*;

#[tokio::test]
async fn done_publishes_help_answered_when_a_help_request_started_the_takeover() {
    let (control, task, _, mut snapshots) = fixture(false);
    let (reply, answer) = oneshot::channel();
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
    control.commands.send(Command::Done).await.unwrap();
    answer.await.unwrap();
    assert!(matches!(
        snapshots.borrow().control,
        ControlState::HandBack {
            result: crate::ipc::HandBackResult::HelpAnswered,
            ..
        }
    ));
    control.stop().await;
    task.await.unwrap();
}

#[tokio::test]
async fn done_publishes_handed_when_no_help_request_started_the_takeover() {
    let (control, task, _, mut snapshots) = fixture(false);
    control.commands.send(Command::TakeControl).await.unwrap();
    taking(&mut snapshots).await;
    control.commands.send(Command::Done).await.unwrap();
    loop {
        if let ControlState::HandBack { result, .. } = snapshots.borrow_and_update().control {
            assert_eq!(result, crate::ipc::HandBackResult::Handed);
            break;
        }
        snapshots.changed().await.unwrap();
    }
    control.stop().await;
    task.await.unwrap();
}
