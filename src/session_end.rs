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
        inbox: &mut Inbox,
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
        inbox.refuse_all(&self.name);
        let stopped =
            call_driver(driver, "stop_recording", json!({}), Duration::from_secs(30)).await;
        // Closing the sandbox drops the window guard and its cause.
        let cause = driver.end_cause();
        let closed = close_driver(driver).await;
        let root = self.folder.root().to_owned();
        match tokio::task::spawn_blocking(move || crate::takeover_clipboard::remove_fifo(&root))
            .await
        {
            Ok(Ok(())) => {}
            error => tracing::warn!(?error, "could not remove the session's clipboard FIFO"),
        }
        let written = write_records(&self.folder, timeline, &summary).await;
        let Some(end) = end else {
            self.scrub_retained().await;
            return Response::WatchEnded;
        };
        let stop = stopped.and_then(check_result);
        let end = end_after_stop(end, &stop, cause);
        if let Err(error) = stop {
            tracing::warn!(%error, "could not stop the recording");
        }
        if let Err(error) = closed {
            tracing::warn!(%error, "could not close the desktop");
        }
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
        let response = match written {
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
                                    crate::title_card::TITLE_CARD.0
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
        };
        if matches!(response, Response::Refused { .. }) {
            self.scrub_retained().await;
        }
        self.publish(SessionState::Ended(Box::new(card)), last_action);
        self.mask_reply(response)
    }
}

fn end_after_stop(
    end: SessionEnd,
    stop: &Result<(), DriverError>,
    cause: Option<crate::checklist::IncompleteReason>,
) -> SessionEnd {
    use crate::checklist::IncompleteReason;
    match (end, cause) {
        (
            SessionEnd::Incomplete(IncompleteReason::DesktopStopped) | SessionEnd::Verdict(_),
            Some(cause),
        ) => SessionEnd::Incomplete(cause),
        (SessionEnd::Verdict(_), None) if stop.is_err() => {
            SessionEnd::Incomplete(IncompleteReason::RecordingNotStopped)
        }
        (end @ (SessionEnd::Verdict(_) | SessionEnd::Incomplete(_)), _) => end,
    }
}
#[cfg(test)]
mod cause_tests {
    use super::*;
    use crate::checklist::{IncompleteReason::*, Verdict};
    #[test]
    fn recording_stop_and_guard_cause_determine_the_end() {
        let verdict = SessionEnd::Verdict(Verdict::Pass);
        let mut cases = vec![
            (
                verdict.clone(),
                Err(DriverError::Arguments),
                Some(HostWindowOpened),
                SessionEnd::Incomplete(HostWindowOpened),
            ),
            (
                verdict.clone(),
                Err(DriverError::Arguments),
                None,
                SessionEnd::Incomplete(RecordingNotStopped),
            ),
            (
                verdict.clone(),
                Ok(()),
                Some(HostWindowOpened),
                SessionEnd::Incomplete(HostWindowOpened),
            ),
            (verdict.clone(), Ok(()), None, verdict),
            (
                SessionEnd::Incomplete(OpenerLeft),
                Err(DriverError::Arguments),
                Some(HostWindowOpened),
                SessionEnd::Incomplete(OpenerLeft),
            ),
            (
                SessionEnd::Incomplete(DesktopStopped),
                Ok(()),
                None,
                SessionEnd::Incomplete(DesktopStopped),
            ),
            (
                SessionEnd::Incomplete(DesktopStopped),
                Ok(()),
                Some(HostWindowOpened),
                SessionEnd::Incomplete(HostWindowOpened),
            ),
            (
                SessionEnd::Incomplete(DesktopStopped),
                Err(DriverError::Arguments),
                Some(HostWindowOpened),
                SessionEnd::Incomplete(HostWindowOpened),
            ),
        ];
        for reason in [
            Idle {
                limit: Duration::from_secs(300),
            },
            OpenerLeft,
            DaemonStopped,
        ] {
            let end = SessionEnd::Incomplete(reason);
            cases.push((end.clone(), Ok(()), Some(HostWindowOpened), end));
        }
        for verdict in [Verdict::Pass, Verdict::NeedsFixes, Verdict::PartlyVerified] {
            cases.push((
                SessionEnd::Verdict(verdict),
                Ok(()),
                Some(HostWindowOpened),
                SessionEnd::Incomplete(HostWindowOpened),
            ));
        }
        for (end, stop, cause, expected) in cases {
            assert_eq!(end_after_stop(end, &stop, cause), expected);
        }
    }
}
