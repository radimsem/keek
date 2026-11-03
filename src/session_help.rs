use super::run::LoopState;
use super::*;
use crate::checklist::IncompleteReason;
impl Actor {
    pub(super) async fn help_request(
        &mut self,
        state: &mut LoopState,
        arrival: gate::Arrival,
        reason: String,
        reply: oneshot::Sender<Response>,
    ) -> Option<IncompleteReason> {
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
        let span = tokio::select! {
                span = &mut waiting => match span {
                    Ok(span) => span,
                    Err(_) => { let _ = reply.send(refusal("The Agent Session ended while the Help Request waited. Open a new Agent Session.")); return Some(IncompleteReason::DesktopStopped); }
                },
                () = self.opener_gone(&mut state.opener) => { let _ = reply.send(refusal("The coding agent disconnected. Open a new Agent Session.")); return Some(IncompleteReason::OpenerLeft);
                }
                () = &mut state.ended => { let _ = reply.send(refusal("The Agent Session ended while the Help Request waited. Open a new Agent Session.")); return Some(IncompleteReason::DesktopStopped); }
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
