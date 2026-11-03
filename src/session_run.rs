use super::*;
impl Actor {
    pub async fn run(
        mut self,
        mut mailbox: mpsc::Receiver<ActorRequest>,
        open_reply: oneshot::Sender<Response>,
    ) -> Finished {
        let opening = Opening {
            placement: self.placement.clone(),
            credentials: self.credentials.clone(),
            project: self.project.clone(),
            worktree: self.worktree.clone(),
            settings: self.settings.clone(),
            name: self.name.clone(),
        };
        let mounts = match self.factory.prepare(&self.folder, &opening).await {
            Ok(mounts) => mounts,
            Err(error) => {
                return Finished {
                    response: self.mask_reply(refusal(error)),
                    name: self.name,
                    reply: Some(open_reply),
                };
            }
        };
        // Outlasts the factory's 30 s handshake limit, whose refusal names the log files.
        let mut driver = match tokio::time::timeout(
            Duration::from_secs(45),
            self.factory.open(&self.folder, &opening, &mounts),
        )
        .await
        .unwrap_or_else(|_| {
            Err(SessionError::Refused(
                "Agent Session setup timed out after 45 s. Check the session logs and retry."
                    .into(),
            ))
        }) {
            Ok(d) => d,
            Err(e) => {
                return Finished {
                    response: self.mask_reply(refusal(e)),
                    name: self.name,
                    reply: Some(open_reply),
                };
            }
        };
        let opened = self.setup(driver.as_mut()).await;
        let zero = match opened {
            Ok(zero) => zero,
            Err(error) => {
                let _ = close_driver(driver.as_mut()).await;
                return Finished {
                    response: self.mask_reply(refusal(error)),
                    name: self.name,
                    reply: Some(open_reply),
                };
            }
        };
        let controller = control::Controller {
            control: self.control.clone(),
            runner: self.runner.clone(),
            compositor: self.compositor.clone(),
            clock: self.clock.clone(),
            events: self.events.clone(),
            opened: self.opened_at,
            window: driver.session_window(),
            project: self.project.clone(),
        };
        let controller_ended = driver.ended();
        let mut controller_task = self
            .control_commands
            .take()
            .map(|commands| tokio::spawn(controller.run(commands, controller_ended)));
        self.publish(SessionState::Open, &None);
        let _ = open_reply.send(Response::Opened {
            credentials: self.credentials.names(),
            warnings: self.warnings.clone(),
            session: self.name.clone(),
            checks: self
                .checklist
                .checks()
                .iter()
                .map(|c| crate::ipc::OpenedCheck {
                    id: c.id,
                    text: c.text.clone(),
                })
                .collect(),
        });
        let mut timeline = SessionTimeline::new(VIDEO_LEAD);
        let mut last_action: Option<LastAction> = None;
        let mut ended = driver.ended();
        loop {
            let request = tokio::select! { biased; () = &mut ended => break, request = mailbox.recv() => { let Some(request) = request else { break; }; request } };
            match request {
                ActorRequest::RequestHelp {
                    arrival,
                    reason,
                    reply,
                } => {
                    let reason = self.credentials.mask(&reason);
                    if reason.trim().is_empty() {
                        let _ = reply.send(refusal(
                            "A Help Request needs a reason. Describe what you need.",
                        ));
                        continue;
                    }
                    let (help_reply, waiting) = oneshot::channel();
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
                        span = waiting => match span { Ok(span) => span, Err(_) => { let _ = reply.send(refusal("The Agent Session ended while the Help Request waited. Open a new Agent Session.")); break; } },
                        () = &mut ended => { let _ = reply.send(refusal("The Agent Session ended while the Help Request waited. Open a new Agent Session.")); break; }
                    };
                    let result = self.hand_back(driver.as_mut(), span).await;
                    let _ = reply.send(self.mask_reply(result));
                }
                ActorRequest::Call {
                    arrival,
                    tool,
                    arguments,
                    reply,
                } => {
                    let _permit = match self.control.gate.enter_for(arrival).await {
                        gate::Entry::Run(permit) => permit,
                        gate::Entry::HandBack(span) => {
                            let result = self.hand_back(driver.as_mut(), span).await;
                            let _ = reply.send(self.mask_reply(result));
                            continue;
                        }
                    };
                    let (mut arguments, action) =
                        match self.catalog.prepare_call(&tool, &self.name, arguments) {
                            Ok(args) => args,
                            Err(e) => {
                                let _ = reply.send(self.mask_reply(refusal(e)));
                                continue;
                            }
                        };
                    if let Err(error) = self.credentials.substitute(&mut arguments) {
                        let _ = reply.send(self.mask_reply(refusal(error)));
                        continue;
                    }
                    let start = self.clock.now();
                    let action_ms = Millis(
                        start
                            .saturating_duration_since(self.opened_at)
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                    );
                    let start_ms = timeline_time(zero, start);
                    let result =
                        call_driver(driver.as_mut(), &tool, arguments, Duration::from_secs(150))
                            .await;
                    let end_ms = timeline_time(zero, self.clock.now());
                    let is_error = result.as_ref().map_or(true, |v| {
                        v.get("isError").and_then(Value::as_bool).unwrap_or(false)
                    });
                    timeline.push(TimelineEvent::Call {
                        tool: tool.clone(),
                        action,
                        start_ms,
                        end_ms,
                        is_error,
                    });
                    if action {
                        last_action = Some(LastAction {
                            tool,
                            time_ms: action_ms,
                        });
                        self.publish(SessionState::Open, &last_action);
                    }
                    let _ = reply.send(self.mask_reply(match result {
                        Ok(value) => Response::ToolResult(value),
                        Err(error) => refusal(error),
                    }));
                }
                ActorRequest::AddCheck { text, reply } => {
                    let response = match self.checklist.add(self.credentials.mask(&text)) {
                        Ok(check) => {
                            self.publish(SessionState::Open, &last_action);
                            Response::CheckAdded { check }
                        }
                        Err(e) => refusal(e),
                    };
                    let _ = reply.send(self.mask_reply(response));
                }
                ActorRequest::MarkCheck {
                    check,
                    outcome,
                    comment,
                    reply,
                } => {
                    let time_ms = timeline_time(zero, self.clock.now());
                    let response = match self.checklist.mark(
                        check,
                        outcome,
                        self.credentials.mask(&comment),
                        time_ms,
                    ) {
                        Ok(()) => {
                            timeline.push(TimelineEvent::CheckMark {
                                check,
                                outcome,
                                at_ms: time_ms,
                            });
                            self.publish(SessionState::Open, &last_action);
                            Response::CheckMarked { check }
                        }
                        Err(e) => refusal(e),
                    };
                    let _ = reply.send(self.mask_reply(response));
                }
                ActorRequest::Close { summary, reply } => {
                    let _permit = loop {
                        match self.control.gate.enter().await {
                            gate::Entry::Run(permit) => break permit,
                            gate::Entry::HandBack(_) => {}
                        }
                    };

                    let summary = self.credentials.mask(&summary);
                    // Everything that can refuse runs before the session stops, so the Verifier can fix it and retry.
                    let ready = match self.checklist.verdict() {
                        Ok(verdict) => crate::evidence::writable_videos_dir(self.runner.as_ref())
                            .await
                            .map(|videos| (verdict, videos))
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    };
                    let (verdict, videos) = match ready {
                        Ok(ready) => ready,
                        Err(message) => {
                            self.closing.store(false, Ordering::SeqCst);
                            let _ = reply.send(self.mask_reply(refusal(message)));
                            continue;
                        }
                    };
                    drop(_permit);
                    self.control.stop().await;
                    if let Some(task) = controller_task.take() {
                        let _ = task.await;
                    }
                    self.record_takeovers(&mut timeline, zero).await;
                    self.publish(SessionState::Closing, &last_action);
                    mailbox.close();
                    while let Ok(queued) = mailbox.try_recv() {
                        let _ = queued.into_reply().send(refusal(format!(
                            "Agent Session {} is closing. Open a new Agent Session.",
                            self.name
                        )));
                    }
                    let stopped = call_driver(
                        driver.as_mut(),
                        "stop_recording",
                        json!({}),
                        Duration::from_secs(30),
                    )
                    .await;
                    let closed = close_driver(driver.as_mut()).await;
                    let written = write_records(&self.folder, &timeline, &summary).await;
                    let stop = stopped.and_then(check_result);
                    let response = match (stop, closed) {
                        (Err(stop), Err(close)) => refusal(format!("{stop}; {close}")),
                        (Err(error), Ok(())) | (Ok(()), Err(error)) => refusal(error),
                        (Ok(()), Ok(())) => match written {
                            Err(e) => refusal(e),
                            Ok(()) => {
                                let edited = crate::editor::edit(
                                    self.runner.as_ref(),
                                    &self.folder.recording(),
                                    &timeline,
                                    &self.folder.root().join("verification.mp4"),
                                    &mut |progress| {
                                        tracing::debug!(?progress, "editing Verification Video")
                                    },
                                )
                                .await;
                                match crate::session_evidence::file(
                                    &self, verdict, videos, summary, edited, &timeline,
                                )
                                .await
                                {
                                    Ok(value) => Response::Closed(value),
                                    Err(message) => refusal(message),
                                }
                            }
                        },
                    };
                    return Finished {
                        response: self.mask_reply(response),
                        name: self.name,
                        reply: Some(reply),
                    };
                }
            }
        }
        self.control.stop().await;
        if let Some(task) = controller_task.take() {
            let _ = task.await;
        }
        self.record_takeovers(&mut timeline, zero).await;
        let _ = call_driver(
            driver.as_mut(),
            "stop_recording",
            json!({}),
            Duration::from_secs(30),
        )
        .await;
        let _ = close_driver(driver.as_mut()).await;
        let _ = write_records(&self.folder, &timeline, "").await;
        Finished {
            name: self.name,
            response: Response::WatchEnded,
            reply: None,
        }
    }
    async fn hand_back(&self, driver: &mut dyn Driver, mut span: gate::Span) -> Response {
        let _permit = loop {
            match self.control.gate.enter().await {
                gate::Entry::Run(permit) => break permit,
                gate::Entry::HandBack(current) => span = current,
            }
        };
        let seconds = span.end.saturating_duration_since(span.start).as_secs();
        let length = format!("{}:{:02}", seconds / 60, seconds % 60);
        let reason = span
            .reason
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default();
        let note = format!(
            "The developer controlled the session for {length}{reason}. The screen may have changed."
        );
        let capture = call_driver(
            driver,
            "get_desktop_state",
            json!({}),
            Duration::from_secs(150),
        )
        .await
        .and_then(|result| {
            if result.get("isError").and_then(Value::as_bool) == Some(true) {
                Err(DriverError::Call {
                    tool: "get_desktop_state".into(),
                    reason: result.to_string(),
                })
            } else {
                Ok(result)
            }
        });
        match capture {
            Ok(mut result) => {
                let mut content = vec![json!({"type":"text", "text":note})];
                if let Some(items) = result.get_mut("content").and_then(Value::as_array_mut) {
                    content.append(items);
                }
                if let Some(object) = result.as_object_mut() {
                    object.insert("content".into(), json!(content));
                } else {
                    result = json!({"content":content});
                }
                Response::ToolResult(result)
            }
            Err(error) => {
                tracing::warn!(%error, "Could not take a fresh screenshot after Takeover");
                Response::ToolResult(
                    json!({"content":[{"type":"text", "text":note}, {"type":"text", "text":"keek couldn't take a fresh screenshot. Take one before you act."}]}),
                )
            }
        }
    }
    async fn record_takeovers(&self, timeline: &mut SessionTimeline, zero: Instant) {
        for span in self.control.drain().await {
            timeline.push(TimelineEvent::Takeover {
                reason: span.reason,
                start_ms: timeline_time(zero, span.start),
                end_ms: timeline_time(zero, span.end),
            });
        }
    }
}
