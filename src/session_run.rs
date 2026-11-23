use super::*;
use crate::timeline::{CheckId, CheckOutcome, PausedSpan};

pub(super) struct LoopState {
    pub(super) driver: Box<dyn Driver>,
    pub(super) opener: Option<watch::Receiver<()>>,
    pub(super) deadline: tokio::time::Instant,
    pub(super) ended: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    pub(super) timeline: SessionTimeline,
    pub(super) last_action: Option<LastAction>,
    pub(super) zero: Instant,
    pub(super) takeover_screen: Option<super::screen::Sample>,
}

impl Actor {
    pub async fn run(self, inbox: Inbox, open_reply: oneshot::Sender<Response>) -> Finished {
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
        let finished = self.run_open(inbox, open_reply).await;
        let _ = stop.send(());
        while watchers.join_next().await.is_some() {}
        if let Some(journal) = journal {
            super::end::finish_journal(journal, &finished, &events).await;
        }
        finished
    }
    async fn run_open(
        mut self,
        mut inbox: Inbox,
        open_reply: oneshot::Sender<Response>,
    ) -> Finished {
        let mut opener = self.opener.take();
        let opening = Opening {
            test_files: self.test_files.folder.clone(),
            placement: self.placement.clone(),
            credentials: self.credentials.clone(),
            project: self.project.clone(),
            worktree: self.worktree.clone(),
            profile: self.profile.clone(),
            name: self.name.clone(),
            progress: self.progress.clone(),
        };
        self.progress.send("Building Project Layer");
        let mounts = match tokio::select! { biased;
            () = opener_left(&mut opener) => {
                Err(SessionError::Refused("The coding agent disconnected during setup. Open a new Agent Session.".into()))
            },
            result = self.factory.prepare(&self.folder, &opening) => result,
        } {
            Ok(mounts) => mounts,
            Err(error) => {
                self.remove_cancelled_folder().await;
                return Finished {
                    response: self.mask_reply(refusal(error)),
                    name: self.name,
                    reply: Some(open_reply),
                };
            }
        };
        self.toolchains = mounts.toolchains.clone();
        if let Some(journal) = &self.journal {
            let profile = crate::report::ProfileSummary::new(&self.profile, &self.toolchains)
                .with_test_files(&self.test_files.list)
                .masked(&self.credentials);
            journal.data.lock().await.profile = profile;
            if let Err(error) = journal.save().await {
                tracing::warn!(%error, "could not record the toolchain versions in the journal");
            }
        }
        self.progress.send("Starting desktop");
        let mut driver = match tokio::select! { biased;
            () = opener_left(&mut opener) => {
                Err(SessionError::Refused("The coding agent disconnected during setup. Open a new Agent Session.".into()))
            },
            result = async { tokio::time::timeout(
            // Outlasts the factory's 30 s handshake limit and 20 s cover wait, whose refusals say more.
            Duration::from_secs(60),
            self.factory.open(&self.folder, &opening, &mounts),
        )
        .await
        .unwrap_or_else(|_| {
            Err(SessionError::Refused(
                "Agent Session setup timed out after 60 s. Open again."
                    .into(),
            ))
        }) } => result,
        } {
            Ok(d) => d,
            Err(e) => {
                self.remove_cancelled_folder().await;
                return Finished {
                    response: self.mask_reply(refusal(e)),
                    name: self.name,
                    reply: Some(open_reply),
                };
            }
        };
        self.progress.send("Starting App Under Test");
        let opened = self.setup(driver.as_mut()).await;
        let zero = match opened {
            Ok(zero) => zero,
            Err(error) => {
                let _ = close_driver(driver.as_mut()).await;
                self.remove_cancelled_folder().await;
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
        let mut control_events = self.events.subscribe();
        let controller_ended = driver.ended();
        let mut controller_task = self
            .control_commands
            .take()
            .map(|commands| tokio::spawn(controller.run(commands, controller_ended)));
        self.publish(SessionState::Open, &None);
        let _ = open_reply.send(Response::Opened {
            files: self.test_files.list.clone(),
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
            takeover_screen: None,
        };
        let mut keepalive = keepalive::timer();
        let mut paused = false;
        let reason = 'requests: loop {
            let request = tokio::select! { biased;
                () = &mut state.ended => break 'requests Some(crate::checklist::IncompleteReason::DesktopStopped),
                () = self.opener_gone(&mut state.opener) => break 'requests Some(crate::checklist::IncompleteReason::OpenerLeft),
                close = Inbox::next_close(&mut inbox.close) => {
                    if let Some(close) = close
                        && let Some((close, ready)) = self.close_when_idle(&mut state, &mut inbox, close).await {
                            return self.finish_close(&mut state, &mut inbox, &mut controller_task, close, ready).await;
                    }
                    continue;
                }
                _ = control_events.changed() => {
                    let control = self.events.borrow().control.clone();
                    let now_paused = control.pauses_idle();
                    if paused && !now_paused { state.deadline = tokio::time::Instant::now() + self.idle_limit; }
                    paused = now_paused;
                    self.sample_takeover(&mut state, &control).await;
                    continue;
                }
                _ = keepalive.tick(), if paused => { keepalive::keep_recording(state.driver.as_mut()).await; continue; }
                () = tokio::time::sleep_until(state.deadline), if !paused && !self.closing.load(Ordering::SeqCst) => { if !self.closing.load(Ordering::SeqCst) { break 'requests Some(crate::checklist::IncompleteReason::Idle { limit: self.idle_limit }); } continue; },
                request = Inbox::next_request(&mut inbox.requests, &mut inbox.deferred) => { let Some(request) = request else { break 'requests None; }; request }
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
                        .forward_call(&mut state, &mut inbox, arrival, tool, arguments, reply)
                        .await
                    {
                        call::CallEnd::Replied => {}
                        call::CallEnd::Close(close, ready) => {
                            return self
                                .finish_close(
                                    &mut state,
                                    &mut inbox,
                                    &mut controller_task,
                                    close,
                                    ready,
                                )
                                .await;
                        }
                        call::CallEnd::End(reason) => break 'requests Some(reason),
                    }
                }
                ActorRequest::AddCheck { text, reply } => {
                    self.add_check(&state.last_action, &state.timeline, text, reply)
                        .await;
                }
                ActorRequest::MarkCheck {
                    check,
                    outcome,
                    comment,
                    reply,
                } => {
                    self.mark_check(
                        &state.last_action,
                        &mut state.timeline,
                        state.zero,
                        check,
                        outcome,
                        comment,
                        reply,
                    )
                    .await;
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
                &mut inbox,
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
        last_action: &Option<LastAction>,
        timeline: &SessionTimeline,
        text: String,
        reply: oneshot::Sender<Response>,
    ) {
        let response = match self.checklist.add(self.credentials.mask(&text)) {
            Ok(check) => {
                self.reminder.reset();
                self.publish(SessionState::Open, last_action);
                Response::CheckAdded { check }
            }
            Err(e) => refusal(e),
        };
        self.persist(timeline).await;
        let _ = reply.send(self.mask_reply(response));
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn mark_check(
        &mut self,
        last_action: &Option<LastAction>,
        timeline: &mut SessionTimeline,
        zero: Instant,
        check: CheckId,
        outcome: CheckOutcome,
        comment: String,
        reply: oneshot::Sender<Response>,
    ) {
        let time_ms = timeline_time(zero, self.clock.now());
        let response =
            match self
                .checklist
                .mark(check, outcome, self.credentials.mask(&comment), time_ms)
            {
                Ok(()) => {
                    self.reminder.reset();
                    timeline.push(TimelineEvent::CheckMark {
                        check,
                        outcome,
                        at_ms: time_ms,
                    });
                    self.publish(SessionState::Open, last_action);
                    Response::CheckMarked { check }
                }
                Err(e) => refusal(e),
            };
        self.persist(timeline).await;
        let _ = reply.send(self.mask_reply(response));
    }
    pub(super) async fn record_takeovers(&self, timeline: &mut SessionTimeline, zero: Instant) {
        for span in self.control.drain().await {
            timeline.push(TimelineEvent::Takeover {
                reason: span.reason,
                start_ms: timeline_time(zero, span.start),
                end_ms: timeline_time(zero, span.end),
                paused: span
                    .paused
                    .into_iter()
                    .map(|pause| PausedSpan {
                        start_ms: timeline_time(zero, pause.start),
                        end_ms: timeline_time(zero, pause.end),
                    })
                    .collect(),
            });
        }
        for help in self.control.drain_help_requests().await {
            timeline.push(TimelineEvent::HelpRequest {
                reason: help.reason,
                start_ms: timeline_time(zero, help.start),
                end_ms: timeline_time(zero, help.end),
                outcome: help.outcome,
            });
        }
    }
}
