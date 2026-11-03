use super::*;
use crate::timeline::{CheckId, CheckOutcome};

pub(super) struct LoopState {
    pub(super) driver: Box<dyn Driver>,
    pub(super) opener: Option<watch::Receiver<()>>,
    pub(super) deadline: tokio::time::Instant,
    pub(super) ended: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    pub(super) timeline: SessionTimeline,
    pub(super) last_action: Option<LastAction>,
    pub(super) zero: Instant,
}

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
        let ended = driver.ended();
        let mut state = LoopState {
            driver,
            opener,
            deadline: tokio::time::Instant::now() + self.idle_limit,
            ended,
            timeline: SessionTimeline::new(VIDEO_LEAD),
            last_action: None,
            zero,
        };
        let mut control_events = self.events.subscribe();
        let reason = 'requests: loop {
            let paused = control_events.borrow().control.pauses_idle();
            let request = tokio::select! { biased;
                () = &mut state.ended => break 'requests Some(crate::checklist::IncompleteReason::DesktopStopped),
                () = self.opener_gone(&mut state.opener) => break 'requests Some(crate::checklist::IncompleteReason::OpenerLeft),
                _ = control_events.changed() => {
                    let now_paused = control_events.borrow().control.pauses_idle();
                    if paused && !now_paused { state.deadline = tokio::time::Instant::now() + self.idle_limit; }
                    continue;
                }
                () = tokio::time::sleep_until(state.deadline), if !paused && !self.closing.load(Ordering::SeqCst) => { if !self.closing.load(Ordering::SeqCst) { break 'requests Some(crate::checklist::IncompleteReason::Idle { limit: self.idle_limit }); } continue; },
                request = mailbox.recv() => { let Some(request) = request else { break 'requests None; }; request }
            };
            state.deadline = tokio::time::Instant::now() + self.idle_limit;
            match request {
                ActorRequest::RequestHelp {
                    arrival,
                    reason,
                    reply,
                } => {
                    if let Some(reason) =
                        self.help_request(&mut state, arrival, reason, reply).await
                    {
                        break 'requests Some(reason);
                    }
                }
                ActorRequest::Call {
                    arrival,
                    tool,
                    arguments,
                    reply,
                } => {
                    match self
                        .forward_call(&mut state, arrival, tool, arguments, reply)
                        .await
                    {
                        call::CallEnd::Replied => {}
                        call::CallEnd::End(reason) => break 'requests Some(reason),
                    }
                }
                ActorRequest::AddCheck { text, reply } => {
                    self.add_check(&mut state, text, reply).await;
                }
                ActorRequest::MarkCheck {
                    check,
                    outcome,
                    comment,
                    reply,
                } => {
                    self.mark_check(&mut state, check, outcome, comment, reply)
                        .await;
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
                            state.driver.as_mut(),
                            &mut mailbox,
                            &mut controller_task,
                            &mut state.timeline,
                            state.zero,
                            &state.last_action,
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
                state.driver.as_mut(),
                &mut mailbox,
                &mut controller_task,
                &mut state.timeline,
                state.zero,
                &state.last_action,
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
    pub(super) async fn add_check(
        &mut self,
        state: &mut LoopState,
        text: String,
        reply: oneshot::Sender<Response>,
    ) {
        let response = match self.checklist.add(self.credentials.mask(&text)) {
            Ok(check) => {
                self.publish(SessionState::Open, &state.last_action);
                Response::CheckAdded { check }
            }
            Err(e) => refusal(e),
        };
        self.persist(&state.timeline).await;
        let _ = reply.send(self.mask_reply(response));
    }
    pub(super) async fn mark_check(
        &mut self,
        state: &mut LoopState,
        check: CheckId,
        outcome: CheckOutcome,
        comment: String,
        reply: oneshot::Sender<Response>,
    ) {
        let time_ms = timeline_time(state.zero, self.clock.now());
        let response =
            match self
                .checklist
                .mark(check, outcome, self.credentials.mask(&comment), time_ms)
            {
                Ok(()) => {
                    state.timeline.push(TimelineEvent::CheckMark {
                        check,
                        outcome,
                        at_ms: time_ms,
                    });
                    self.publish(SessionState::Open, &state.last_action);
                    Response::CheckMarked { check }
                }
                Err(e) => refusal(e),
            };
        self.persist(&state.timeline).await;
        let _ = reply.send(self.mask_reply(response));
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
