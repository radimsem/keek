use super::*;

#[tokio::test]
async fn marking_a_help_request_unanswered_only_changes_the_matching_takeover() {
    let (control, task, _, mut snapshots) = fixture(false);
    let mut starts = Vec::new();
    for reason in ["First step", "Second step"] {
        let (reply, response) = oneshot::channel();
        control
            .commands
            .send(Command::Help {
                arrival: control.gate.arrival(),
                reason: reason.into(),
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
        starts.push(response.await.unwrap().start);
    }
    control.mark_unanswered(starts[0]).await;
    control.mark_unanswered(starts[0]).await;
    control.stop().await;
    task.await.unwrap();

    let help = control.drain_help_requests().await;
    assert_eq!(help.len(), 2);
    assert_eq!(help[0].end, starts[0]);
    assert_eq!(help[0].outcome, HelpOutcome::Unanswered);
    assert_eq!(help[1].end, starts[1]);
    assert_eq!(help[1].outcome, HelpOutcome::Answered);
}
