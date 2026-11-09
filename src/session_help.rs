use super::run::LoopState;
use super::*;
use crate::checklist::IncompleteReason;
impl Actor {
    pub(super) async fn help_request(
        &mut self,
        state: &mut LoopState,
        arrival: gate::Arrival,
        reason: String,
        mut reply: oneshot::Sender<Response>,
    ) -> Option<IncompleteReason> {
        if reply.is_closed() {
            return None;
        }
        let reason = self.credentials.mask(&reason);
        if reason.trim().is_empty() {
            let _ = reply.send(refusal(
                "A Help Request needs a reason. Describe what you need.",
            ));
            return None;
        }
        let (help_reply, mut waiting) = oneshot::channel();
        let _ = self
            .control
            .commands
            .send(control::Command::Help {
                arrival,
                reason,
                reply: help_reply,
            })
            .await;
        let mut keepalive = keepalive::timer();
        let span = loop {
            tokio::select! {
                    span = &mut waiting => break match span {
                        Ok(span) => span,
                        Err(_) => { let _ = reply.send(refusal("The Agent Session ended while the Help Request waited. Open a new Agent Session.")); return Some(IncompleteReason::DesktopStopped); }
                    },
                    () = reply.closed() => {
                        drop(waiting);
                        let (acknowledged, restored) = oneshot::channel();
                        if self
                            .control
                            .commands
                            .send(control::Command::Withdraw(acknowledged))
                            .await
                            .is_ok()
                        {
                            let _ = restored.await;
                        }
                        state.deadline = tokio::time::Instant::now() + self.idle_limit;
                        return None;
                    }
                    () = self.opener_gone(&mut state.opener) => { let _ = reply.send(refusal("The coding agent disconnected. Open a new Agent Session.")); return Some(IncompleteReason::OpenerLeft);
                    }
                    () = &mut state.ended => { let _ = reply.send(refusal("The Agent Session ended while the Help Request waited. Open a new Agent Session.")); return Some(IncompleteReason::DesktopStopped); }
                    _ = keepalive.tick() => keepalive::keep_recording(state.driver.as_mut()).await,
            }
        };
        let result = match self
            .hand_back(
                state.driver.as_mut(),
                span,
                &mut state.opener,
                &mut state.deadline,
            )
            .await
        {
            Ok(response) => response,
            Err(reason) => {
                let _ = reply.send(refusal(reason.refusal()));
                return Some(reason);
            }
        };
        let _ = reply.send(self.mask_reply(result));
        None
    }
}
