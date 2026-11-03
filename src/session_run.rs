use super::*;
impl Actor {
    pub async fn run(
        self,
        mailbox: mpsc::Receiver<ActorRequest>,
        open_reply: oneshot::Sender<Response>,
    ) -> Finished {
        let journal = self.journal.clone();
        let events = self.events.subscribe();
        let (stop, stopped) = oneshot::channel();
        let mut watchers = tokio::task::JoinSet::new();
        if let Some(handle) = &journal {
            watchers.spawn(super::journal_watch::watch_control(
                handle.clone(),
                self.events.subscribe(),
                stopped,
                self.clock.clone(),
                self.opened_at,
            ));
        }
        let finished = self.run_open(mailbox, open_reply).await;
        let _ = stop.send(());
        while watchers.join_next().await.is_some() {}
        if let Some(journal) = journal {
            super::end::finish_journal(journal, &finished, &events).await;
        }
        finished
    }
    async fn run_open(
        mut self,
        mut mailbox: mpsc::Receiver<ActorRequest>,
        open_reply: oneshot::Sender<Response>,
    ) -> Finished {
        let mut opener = self.opener.take();
        let opening = Opening {
            placement: self.placement.clone(),
            credentials: self.credentials.clone(),
            project: self.project.clone(),
            worktree: self.worktree.clone(),
            settings: self.settings.clone(),
            name: self.name.clone(),
        };
        let mounts = match tokio::select! { biased;
            () = opener_left(&mut opener) => {
                self.remove_cancelled_folder().await;
                Err(SessionError::Refused("The coding agent disconnected during setup. Open a new Agent Session.".into()))
            },
            result = self.factory.prepare(&self.folder, &opening) => result,
        } {
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
        let mut driver = match tokio::select! { biased;
            () = opener_left(&mut opener) => {
                self.remove_cancelled_folder().await;
                Err(SessionError::Refused("The coding agent disconnected during setup. Open a new Agent Session.".into()))
            },
            result = async { tokio::time::timeout(
            Duration::from_secs(45),
            self.factory.open(&self.folder, &opening, &mounts),
        )
        .await
        .unwrap_or_else(|_| {
            Err(SessionError::Refused(
                "Agent Session setup timed out after 45 s. Check the session logs and retry."
                    .into(),
            ))
        }) } => result,
        } {
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
        if let Some(journal) = &self.journal {
            journal.recording_offset.store(
                zero.saturating_duration_since(self.opened_at).as_millis() as u64 + VIDEO_LEAD.0,
                Ordering::SeqCst,
            );
        }
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
        let mut control_events = self.events.subscribe();
        let mut deadline = tokio::time::Instant::now() + self.idle_limit;
        let reason = 'requests: loop {
            let paused = control_events.borrow().control.pauses_idle();
            let request = tokio::select! { biased;
                () = &mut ended => break 'requests Some(crate::checklist::IncompleteReason::DesktopStopped),
                // Close can be queued after select checks its guards.
                () = opener_left(&mut opener), if !self.closing.load(Ordering::SeqCst) => { if !self.closing.load(Ordering::SeqCst) { break 'requests Some(crate::checklist::IncompleteReason::OpenerLeft); } continue; },
                _ = control_events.changed() => {
                    let now_paused = control_events.borrow().control.pauses_idle();
                    if paused && !now_paused { deadline = tokio::time::Instant::now() + self.idle_limit; }
                    continue;
                }
                () = tokio::time::sleep_until(deadline), if !paused && !self.closing.load(Ordering::SeqCst) => { if !self.closing.load(Ordering::SeqCst) { break 'requests Some(crate::checklist::IncompleteReason::Idle { limit: self.idle_limit }); } continue; },
                request = mailbox.recv() => { let Some(request) = request else { break 'requests None; }; request }
            };
            deadline = tokio::time::Instant::now() + self.idle_limit;
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
                    let span = loop {
                        tokio::select! {
                            span = &mut waiting => match span {
                                Ok(span) => break span,
                                Err(_) => { let _ = reply.send(refusal("The Agent Session ended while the Help Request waited. Open a new Agent Session.")); break 'requests Some(crate::checklist::IncompleteReason::DesktopStopped); }
                            },
                            () = opener_left(&mut opener), if !self.closing.load(Ordering::SeqCst) => {
                                if !self.closing.load(Ordering::SeqCst) { let _ = reply.send(refusal("The coding agent disconnected. Open a new Agent Session.")); break 'requests Some(crate::checklist::IncompleteReason::OpenerLeft); }
                            }
                            () = &mut ended => { let _ = reply.send(refusal("The Agent Session ended while the Help Request waited. Open a new Agent Session.")); break 'requests Some(crate::checklist::IncompleteReason::DesktopStopped); }
                        }
                    };
                    let result = match self
                        .hand_back(driver.as_mut(), span, &mut opener, &mut deadline)
                        .await
                    {
                        Ok(response) => response,
                        Err(reason) => {
                            let _ = reply.send(refusal(reason.refusal()));
                            break 'requests Some(reason);
                        }
                    };
                    let _ = reply.send(self.mask_reply(result));
                }
                ActorRequest::Call {
                    arrival,
                    tool,
                    arguments,
                    reply,
                } => {
                    let entry = tokio::select! {
                        entry = self.control.gate.enter_for(arrival) => entry,
                        () = opener_left(&mut opener), if !self.closing.load(Ordering::SeqCst) => { if !self.closing.load(Ordering::SeqCst) { break 'requests Some(crate::checklist::IncompleteReason::OpenerLeft); } continue; },
                    };
                    let _permit = match entry {
                        gate::Entry::Run(permit) => permit,
                        gate::Entry::HandBack(span) => {
                            let result = match self
                                .hand_back(driver.as_mut(), span, &mut opener, &mut deadline)
                                .await
                            {
                                Ok(response) => response,
                                Err(reason) => {
                                    let _ = reply.send(refusal(reason.refusal()));
                                    break 'requests Some(reason);
                                }
                            };
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
                    let interrupt = |reason: crate::checklist::IncompleteReason| {
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
                            driver.as_mut(),
                            &tool,
                            arguments,
                            Duration::from_secs(150),
                        );
                        tokio::pin!(calling);
                        loop {
                            tokio::select! { biased;
                                () = &mut ended => break interrupt(crate::checklist::IncompleteReason::DesktopStopped),
                                result = &mut calling => break (result, None),
                                () = opener_left(&mut opener), if !self.closing.load(Ordering::SeqCst) => {
                                    if !self.closing.load(Ordering::SeqCst) { break interrupt(crate::checklist::IncompleteReason::OpenerLeft); }
                                }
                            }
                        }
                    };
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
                    self.persist(&timeline).await;
                    deadline = tokio::time::Instant::now() + self.idle_limit;
                    let _ = reply.send(self.mask_reply(if let Some(reason) = interrupted {
                        refusal(reason.refusal())
                    } else {
                        match result {
                            Ok(value) => Response::ToolResult(value),
                            Err(error) => refusal(error),
                        }
                    }));
                    if let Some(reason) = interrupted {
                        break 'requests Some(reason);
                    }
                }
                ActorRequest::AddCheck { text, reply } => {
                    let response = match self.checklist.add(self.credentials.mask(&text)) {
                        Ok(check) => {
                            self.publish(SessionState::Open, &last_action);
                            Response::CheckAdded { check }
                        }
                        Err(e) => refusal(e),
                    };
                    self.persist(&timeline).await;
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
                    self.persist(&timeline).await;
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
                    let response = self
                        .end_session(
                            driver.as_mut(),
                            &mut mailbox,
                            &mut controller_task,
                            &mut timeline,
                            zero,
                            &last_action,
                            Some(crate::checklist::SessionEnd::Verdict(verdict)),
                            Ok(videos),
                            summary,
                        )
                        .await;
                    return Finished {
                        response: self.mask_reply(response),
                        name: self.name,
                        reply: Some(reply),
                    };
                }
            }
        };
        let videos = match reason {
            Some(_) => crate::evidence::writable_videos_dir(self.runner.as_ref())
                .await
                .map_err(|e| e.to_string()),
            None => Ok(PathBuf::new()),
        };
        let response = self
            .end_session(
                driver.as_mut(),
                &mut mailbox,
                &mut controller_task,
                &mut timeline,
                zero,
                &last_action,
                reason.map(crate::checklist::SessionEnd::Incomplete),
                videos,
                String::new(),
            )
            .await;
        Finished {
            name: self.name,
            response,
            reply: None,
        }
    }
    pub(super) async fn record_takeovers(&self, timeline: &mut SessionTimeline, zero: Instant) {
        for span in self.control.drain().await {
            timeline.push(TimelineEvent::Takeover {
                reason: span.reason,
                start_ms: timeline_time(zero, span.start),
                end_ms: timeline_time(zero, span.end),
            });
        }
    }
}
