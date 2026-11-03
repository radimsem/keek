use super::run::LoopState;
use super::*;
use crate::checklist::IncompleteReason;
pub(super) enum CallEnd {
    Replied,
    End(IncompleteReason),
    Close(CloseRequest, super::close::ReadyClose),
}
impl Actor {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn forward_call(
        &mut self,
        state: &mut LoopState,
        inbox: &mut Inbox,
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
        let mut accepted_close = None;
        let (result, interrupted) = {
            let calling = call_driver(
                state.driver.as_mut(),
                &tool,
                arguments,
                Duration::from_secs(150),
            );
            tokio::pin!(calling);
            loop {
                tokio::select! { biased;
                        () = &mut state.ended => break interrupt(IncompleteReason::DesktopStopped),
                        close = Inbox::next_close(&mut inbox.close) => {
                            if let Some(close) = close
                                && let Some(ready) = self.prepare_close(inbox, &mut state.timeline, &state.last_action, state.zero, &mut state.deadline, close).await {
                                    accepted_close = Some(ready);
                                    break (Err(DriverError::Call { tool: tool.clone(), reason: "session is closing".into() }), None);
                            }
                        }
                        result = &mut calling => break (result, None),
                        () = self.opener_gone(&mut state.opener) => break interrupt(IncompleteReason::OpenerLeft),
                }
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
                tool: tool.clone(),
                time_ms: action_ms,
            });
            self.publish(SessionState::Open, &state.last_action);
        }
        self.persist(&state.timeline).await;
        state.deadline = tokio::time::Instant::now() + self.idle_limit;
        let _ = reply.send(self.mask_reply(if accepted_close.is_some() {
            refusal(format!(
                "Agent Session {} is closing, so {tool} was stopped. Open a new Agent Session.",
                self.name
            ))
        } else if let Some(reason) = interrupted {
            refusal(reason.refusal())
        } else {
            match result {
                Ok(value) => Response::ToolResult(value),
                Err(error) => refusal(error),
            }
        }));
        if let Some((close, ready)) = accepted_close {
            drop(_permit);
            return CallEnd::Close(close, ready);
        }
        if let Some(reason) = interrupted {
            return CallEnd::End(reason);
        }
        CallEnd::Replied
    }
}
