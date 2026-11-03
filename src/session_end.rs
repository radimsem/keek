use super::*;
use crate::checklist::SessionEnd;
pub(super) async fn finish_journal(
    journal: crate::session_journal::Handle,
    finished: &Finished,
    events: &watch::Receiver<SessionSnapshot>,
) {
    if let (Response::Refused { message }, SessionState::Ended(_)) =
        (&finished.response, &events.borrow().state)
    {
        tracing::error!(%message, "session Evidence could not be filed; retaining its journal");
        return;
    }
    journal.remove().await;
}
impl Actor {
    async fn scrub_retained(&self) {
        let folder = self.folder.root().to_owned();
        let credentials = self.credentials.clone();
        let scrub = tokio::task::spawn_blocking(move || {
            crate::credential_scrub::scrub_folder(&credentials, &folder)
        })
        .await;
        if !matches!(scrub, Ok(Ok(()))) {
            tracing::error!(?scrub, "could not scrub the retained Action Log");
        }
    }
    pub(super) async fn remove_cancelled_folder(&self) {
        let folder = self.folder.root().to_owned();
        let result = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(folder)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            error => tracing::error!(?error, "could not remove the cancelled session folder"),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn end_session(
        &self,
        driver: &mut dyn Driver,
        mailbox: &mut mpsc::Receiver<ActorRequest>,
        controller_task: &mut Option<tokio::task::JoinHandle<()>>,
        timeline: &mut SessionTimeline,
        zero: Instant,
        last_action: &Option<LastAction>,
        end: Option<SessionEnd>,
        videos: Result<PathBuf, String>,
        summary: String,
    ) -> Response {
        self.closing.store(true, Ordering::SeqCst);
        self.control.stop().await;
        if let Some(task) = controller_task.take() {
            let _ = task.await;
        }
        self.record_takeovers(timeline, zero).await;
        self.persist(timeline).await;
        if end.is_some() {
            self.publish(SessionState::Closing, last_action);
        }
        mailbox.close();
        while let Ok(queued) = mailbox.try_recv() {
            let _ = queued.into_reply().send(refusal(format!(
                "Agent Session {} is closing. Open a new Agent Session.",
                self.name
            )));
        }
        let stopped =
            call_driver(driver, "stop_recording", json!({}), Duration::from_secs(30)).await;
        let closed = close_driver(driver).await;
        let written = write_records(&self.folder, timeline, &summary).await;
        let Some(end) = end else {
            self.scrub_retained().await;
            return Response::WatchEnded;
        };
        let elapsed = Millis(self.clock.now().duration_since(self.opened_at).as_millis() as u64);
        let frame = crate::session_frame::capture(
            self.runner.as_ref(),
            &self.folder.recording(),
            &self.frames_dir,
            &self.name,
        )
        .await;
        let mut card = crate::ipc::EndedSession {
            end: end.clone(),
            revision: self.revision.clone(),
            attempt: None,
            verification: None,
            folder: None,
            video_length: Millis(0),
            actions: timeline
                .events()
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        crate::timeline::TimelineEvent::Call { action: true, .. }
                    )
                })
                .count(),
            frame: frame.clone(),
            elapsed,
        };
        let stop = stopped.and_then(check_result);
        let (stop, closed) = match &end {
            SessionEnd::Verdict(_) => (stop, closed),
            SessionEnd::Incomplete(_) => {
                if let Err(error) = stop {
                    tracing::warn!(%error, "recording stopped before the session ended");
                }
                if let Err(error) = closed {
                    tracing::warn!(%error, "desktop stopped before the session ended");
                }
                (Ok(()), Ok(()))
            }
        };
        let response = match (stop, closed) {
            (Err(stop), Err(close)) => refusal(format!("{stop}; {close}")),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => refusal(error),
            (Ok(()), Ok(())) => match written {
                Err(e) => refusal(e),
                Ok(()) => {
                    let output = self.folder.root().join("verification.mp4");
                    let mut last_percent = crate::ipc::Percent(0);
                    self.publish(
                        SessionState::Rendering {
                            percent: last_percent,
                            frame: frame.clone(),
                            elapsed,
                        },
                        last_action,
                    );
                    let mut progress = |progress: crate::editor::EditProgress| {
                        if progress.percent != last_percent {
                            last_percent = progress.percent;
                            self.publish(
                                SessionState::Rendering {
                                    percent: last_percent,
                                    frame: frame.clone(),
                                    elapsed,
                                },
                                last_action,
                            );
                        }
                    };
                    let edited = match &end {
                        SessionEnd::Verdict(_) => {
                            crate::editor::edit(
                                self.runner.as_ref(),
                                &self.folder.recording(),
                                timeline,
                                &output,
                                &mut progress,
                            )
                            .await
                        }
                        SessionEnd::Incomplete(reason) => {
                            crate::editor::edit_incomplete(
                                self.runner.as_ref(),
                                &self.folder.recording(),
                                timeline,
                                &output,
                                &mut progress,
                                reason,
                            )
                            .await
                        }
                    };
                    card.video_length = edited
                        .as_ref()
                        .map(|plan| {
                            Millis(
                                plan.output_length().0
                                    + if matches!(end, SessionEnd::Incomplete(_)) {
                                        crate::title_card::TAKEOVER_CARD.0
                                    } else {
                                        0
                                    },
                            )
                        })
                        .unwrap_or(Millis(0));
                    match crate::session_evidence::file(
                        self.into(),
                        end,
                        videos,
                        summary,
                        edited,
                        timeline,
                    )
                    .await
                    {
                        Ok(value) => {
                            card.verification = value["evidence"]["verification"]
                                .as_str()
                                .map(PathBuf::from);
                            card.folder = value["evidence"]["report"]
                                .as_str()
                                .and_then(|path| std::path::Path::new(path).parent())
                                .map(std::path::Path::to_owned);
                            if let Some(index) = value["evidence"]["index"].as_str() {
                                let index = PathBuf::from(index);
                                let branch = self.revision.branch.clone();
                                card.attempt = tokio::task::spawn_blocking(move || {
                                    std::fs::read_to_string(index).ok().and_then(|text| {
                                        crate::attempts::count(&text, branch.as_deref())
                                    })
                                })
                                .await
                                .ok()
                                .flatten();
                            }
                            Response::Closed(value)
                        }
                        Err(message) => refusal(message),
                    }
                }
            },
        };
        if matches!(response, Response::Refused { .. }) {
            self.scrub_retained().await;
        }
        self.publish(SessionState::Ended(Box::new(card)), last_action);
        self.mask_reply(response)
    }
}
