use super::run::LoopState;
use super::*;
use crate::checklist::IncompleteReason;
pub(super) enum CallEnd {
    Replied,
    End(IncompleteReason),
}
impl Actor {
    pub(super) async fn forward_call(
        &mut self,
        state: &mut LoopState,
        arrival: gate::Arrival,
        tool: String,
        arguments: Value,
        reply: oneshot::Sender<Response>,
    ) -> CallEnd {
        let entry = tokio::select! {
            entry = self.control.gate.enter_for(arrival) => entry,
            () = self.opener_gone(&mut state.opener) => return CallEnd::End(IncompleteReason::OpenerLeft),
        };
        let _permit = match entry {
            gate::Entry::Run(permit) => permit,
            gate::Entry::HandBack(span) => {
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
                        return CallEnd::End(reason);
                    }
                };
                let _ = reply.send(self.mask_reply(result));
                return CallEnd::Replied;
            }
        };
        let (mut arguments, action) = match self.catalog.prepare_call(&tool, &self.name, arguments)
        {
            Ok(args) => args,
            Err(e) => {
                let _ = reply.send(self.mask_reply(refusal(e)));
                return CallEnd::Replied;
            }
        };
        if let Err(error) = self.credentials.substitute(&mut arguments) {
            let _ = reply.send(self.mask_reply(refusal(error)));
            return CallEnd::Replied;
        }
        let start = self.clock.now();
        let action_ms = Millis(
            start
                .saturating_duration_since(self.opened_at)
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        );
        let start_ms = timeline_time(state.zero, start);
        let interrupt = |reason: IncompleteReason| {
            (
                Err(DriverError::Call {
                    tool: tool.clone(),
                    reason: reason.sentence(),
                }),
                Some(reason),
            )
        };
        let (result, interrupted) = {
            let calling = call_driver(
                state.driver.as_mut(),
                &tool,
                arguments,
                Duration::from_secs(150),
            );
            tokio::pin!(calling);
            tokio::select! { biased;
                    () = &mut state.ended => interrupt(IncompleteReason::DesktopStopped),
                    result = &mut calling => (result, None),
                    () = self.opener_gone(&mut state.opener) => interrupt(IncompleteReason::OpenerLeft),
            }
        };
        let end_ms = timeline_time(state.zero, self.clock.now());
        let is_error = result.as_ref().map_or(true, |v| {
            v.get("isError").and_then(Value::as_bool).unwrap_or(false)
        });
        state.timeline.push(TimelineEvent::Call {
            tool: tool.clone(),
            action,
            start_ms,
            end_ms,
            is_error,
        });
        if action {
            state.last_action = Some(LastAction {
                tool,
                time_ms: action_ms,
            });
            self.publish(SessionState::Open, &state.last_action);
        }
        self.persist(&state.timeline).await;
        state.deadline = tokio::time::Instant::now() + self.idle_limit;
        let _ = reply.send(self.mask_reply(if let Some(reason) = interrupted {
            refusal(reason.refusal())
        } else {
            match result {
                Ok(value) => Response::ToolResult(value),
                Err(error) => refusal(error),
            }
        }));
        if let Some(reason) = interrupted {
            return CallEnd::End(reason);
        }
        CallEnd::Replied
    }
}
