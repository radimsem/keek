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
        let mut control = self.events.subscribe();
        let current = self.events.borrow().control.clone();
        self.sample_takeover(state, &current).await;
        let mut keepalive = keepalive::timer();
        let entry = loop {
            tokio::select! {
                entry = self.control.gate.enter_for(arrival) => break entry,
                () = self.opener_gone(&mut state.opener) => return CallEnd::End(IncompleteReason::OpenerLeft),
                _ = keepalive.tick() => keepalive::keep_recording(state.driver.as_mut()).await,
                _ = control.changed() => {
                    let current = self.events.borrow().control.clone();
                    self.sample_takeover(state, &current).await;
                }
            }
        };
        let _permit = match entry {
            gate::Entry::Run(permit) => permit,
            gate::Entry::HandBack(span) => {
                let result = match self
                    .hand_back(
                        state.driver.as_mut(),
                        span,
                        &state.takeover_screen,
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
        self.events
            .send_if_modified(|snapshot| std::mem::take(&mut snapshot.reminded));
        if tool == super::delegate::TOOL {
            let (response, end) = self.delegate_step(state, inbox, arguments).await;
            let _ = reply.send(self.mask_reply(response));
            return end;
        }
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
        let action_ms = self.control_time(start);
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
        let raced = self
            .race(state, inbox, |driver| {
                call_driver(driver, &tool, arguments, DRIVER_LIMIT)
            })
            .await;
        let (result, interrupted) = match raced {
            Raced::Finished(result) => (result, None),
            Raced::Ended(reason) => interrupt(reason),
            Raced::Closing(close, ready) => {
                accepted_close = Some((close, ready));
                (
                    Err(DriverError::Call {
                        tool: tool.clone(),
                        reason: "session is closing".into(),
                    }),
                    None,
                )
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
        state.deadline = tokio::time::Instant::now() + self.idle_limit;
        let result = if interrupted.is_none() && accepted_close.is_none() {
            match result {
                Ok(value) => {
                    let mut value = refusal_hint::add(state.driver.as_mut(), value).await;
                    if self.reminder.record_call(&self.checklist, &mut value) {
                        state
                            .timeline
                            .push(TimelineEvent::Reminder { at_ms: end_ms });
                        self.events.send_modify(|snapshot| snapshot.reminded = true);
                    }
                    Ok(value)
                }
                Err(error) => Err(error),
            }
        } else {
            result
        };
        self.persist(&state.timeline).await;
        let _ = reply.send(self.mask_reply(if accepted_close.is_some() {
            self.closing_refusal(&tool)
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

    /// Runs the future `start` makes until it finishes, the desktop stops, the opener leaves or a
    /// close is accepted. A close that is refused leaves the future running.
    pub(super) async fn race<'s, F: Future>(
        &mut self,
        state: &'s mut LoopState,
        inbox: &mut Inbox,
        start: impl FnOnce(&'s mut dyn Driver) -> F,
    ) -> Raced<F::Output> {
        let LoopState {
            driver,
            opener,
            deadline,
            ended,
            timeline,
            last_action,
            zero,
            ..
        } = state;
        let calling = start(driver.as_mut());
        tokio::pin!(calling);
        loop {
            tokio::select! { biased;
                () = &mut *ended => break Raced::Ended(IncompleteReason::DesktopStopped),
                close = Inbox::next_close(&mut inbox.close) => {
                    if let Some(close) = close
                        && let Some((close, ready)) = self.prepare_close(inbox, timeline, last_action, *zero, deadline, close).await {
                            break Raced::Closing(close, ready);
                    }
                }
                output = &mut calling => break Raced::Finished(output),
                () = self.opener_gone(opener) => break Raced::Ended(IncompleteReason::OpenerLeft),
            }
        }
    }

    pub(super) fn closing_refusal(&self, tool: &str) -> Response {
        refusal(format!(
            "Agent Session {} is closing, so {tool} was stopped. Open a new Agent Session.",
            self.name
        ))
    }
}

pub(super) enum Raced<T> {
    Finished(T),
    Ended(IncompleteReason),
    Closing(CloseRequest, super::close::ReadyClose),
}
