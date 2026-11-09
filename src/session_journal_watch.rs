use super::*;
impl Actor {
    pub(super) async fn persist(&self, timeline: &SessionTimeline) {
        if let Some(handle) = &self.journal {
            let mut data = handle.data.lock().await;
            data.checks = self.checklist.checks().to_vec();
            data.timeline = timeline.clone();
            if let Err(error) = handle.write(&data).await {
                tracing::error!(%error, "could not save session journal");
            }
        }
    }
}

pub(super) async fn watch_control(
    handle: crate::session_journal::Handle,
    mut events: watch::Receiver<SessionSnapshot>,
    mut stop: oneshot::Receiver<()>,
    clock: Arc<dyn Clock + Send + Sync>,
    opened: Instant,
) {
    loop {
        let state = events.borrow_and_update().control.clone();
        let mut data = handle.data.lock().await;
        let offset = handle.recording_offset.load(Ordering::SeqCst);
        let now = Millis(
            (clock.now().saturating_duration_since(opened).as_millis() as u64)
                .saturating_sub(offset),
        );
        let help = track_help(&mut data.help_requests, &state, offset, now);
        let takeover = match state {
            crate::ipc::ControlState::Takeover { start_ms, .. } => {
                let start = Millis(start_ms.0.saturating_sub(offset));
                if !data.takeovers.iter().any(|span| span.start == start) {
                    data.takeovers
                        .push(crate::session_journal::Takeover { start, end: None });
                    true
                } else {
                    false
                }
            }
            crate::ipc::ControlState::HandBack { end_ms, length } => {
                let end = Millis(end_ms.0.saturating_sub(offset));
                let start = Millis(end_ms.0.saturating_sub(length.0).saturating_sub(offset));
                if let Some(span) = data
                    .takeovers
                    .iter_mut()
                    .rev()
                    .find(|span| span.end.is_none())
                {
                    span.end = Some(end);
                    true
                } else if data
                    .takeovers
                    .last()
                    .is_some_and(|span| span.end == Some(end))
                {
                    false
                } else {
                    data.takeovers.push(crate::session_journal::Takeover {
                        start,
                        end: Some(end),
                    });
                    true
                }
            }
            crate::ipc::ControlState::Verifier | crate::ipc::ControlState::HelpRequest { .. } => {
                if let Some(span) = data.takeovers.last_mut()
                    && span.end.is_none()
                {
                    span.end = Some(now);
                    true
                } else {
                    false
                }
            }
        };
        let changed = help || takeover;
        if changed && let Err(error) = handle.write(&data).await {
            tracing::error!(%error, "could not save Takeover journal");
        }
        drop(data);
        tokio::select! {
            _ = &mut stop => break,
            changed = events.changed() => if changed.is_err() { break; },
        }
    }
}

/// Opens a journaled Help Request when the card shows one and closes it on Take control or
/// withdrawal. Returns whether the journal changed.
fn track_help(
    help: &mut Vec<crate::session_journal::HelpRequest>,
    state: &crate::ipc::ControlState,
    offset: u64,
    now: Millis,
) -> bool {
    let open = help.last_mut().filter(|request| request.end.is_none());
    match (state, open) {
        (crate::ipc::ControlState::HelpRequest { reason, start_ms }, None) => {
            help.push(crate::session_journal::HelpRequest {
                reason: reason.clone(),
                start: Millis(start_ms.0.saturating_sub(offset)),
                end: None,
            });
            true
        }
        (crate::ipc::ControlState::Takeover { start_ms, .. }, Some(request)) => {
            request.end = Some((
                Millis(start_ms.0.saturating_sub(offset)),
                crate::timeline::HelpOutcome::Answered,
            ));
            true
        }
        (
            crate::ipc::ControlState::Verifier | crate::ipc::ControlState::HandBack { .. },
            Some(request),
        ) => {
            request.end = Some((now, crate::timeline::HelpOutcome::Withdrawn));
            true
        }
        (crate::ipc::ControlState::HelpRequest { .. }, Some(_))
        | (
            crate::ipc::ControlState::Verifier
            | crate::ipc::ControlState::Takeover { .. }
            | crate::ipc::ControlState::HandBack { .. },
            None,
        ) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn hand_back_closes_the_open_takeover_when_times_round_differently() {
        let root = tempfile::tempdir().unwrap();
        let journal = crate::session_journal::Journal {
            project: "app".into(),
            name: "app_12-00-00".parse().unwrap(),
            started: String::new(),
            revision: crate::revision::Revision {
                branch: None,
                commit: String::new(),
                dirty: false,
            },
            goal: String::new(),
            checks: vec![],
            timeline: SessionTimeline::new(Millis(0)),
            takeovers: vec![],
            help_requests: vec![],
            credentials_hash: None,
            profile: Default::default(),
        };
        let handle = crate::session_journal::Handle::create(root.path(), journal)
            .await
            .unwrap();
        handle.recording_offset.store(0, Ordering::SeqCst);
        let (events, receiver) = watch::channel(SessionSnapshot {
            control: crate::ipc::ControlState::Takeover {
                start_ms: Millis(1000),
                shortcuts: crate::ipc::HostShortcuts::Captured,
            },
            state: SessionState::Open,
            checks: vec![],
            last_action: None,
        });
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(watch_control(
            handle.clone(),
            receiver,
            stopped,
            Arc::new(crate::clock::MonotonicClock),
            Instant::now(),
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while handle.data.lock().await.takeovers.is_empty() {
                tokio::task::yield_now().await;
            }
            events.send_modify(|snapshot| {
                snapshot.control = crate::ipc::ControlState::HandBack {
                    end_ms: Millis(2001),
                    length: Millis(1000),
                }
            });
            loop {
                let spans = handle.data.lock().await.takeovers.clone();
                if spans.iter().any(|span| span.end.is_some()) {
                    assert_eq!(spans.len(), 1);
                    assert_eq!(spans[0].end, Some(Millis(2001)));
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let _ = stop.send(());
        task.await.unwrap();
    }

    fn journal_after(states: &[crate::ipc::ControlState]) -> crate::session_journal::Journal {
        let mut journal = crate::session_journal::Journal {
            project: "app".into(),
            name: "app_12-00-00".parse().unwrap(),
            started: String::new(),
            revision: crate::revision::Revision {
                branch: None,
                commit: String::new(),
                dirty: false,
            },
            goal: String::new(),
            checks: vec![],
            timeline: SessionTimeline::new(Millis(0)),
            takeovers: vec![],
            help_requests: vec![],
            credentials_hash: None,
            profile: Default::default(),
        };
        for state in states {
            track_help(&mut journal.help_requests, state, 100, Millis(9_900));
        }
        journal
    }

    #[test]
    fn a_journaled_help_request_ends_where_its_takeover_starts() {
        let mut journal = journal_after(&[
            crate::ipc::ControlState::HelpRequest {
                reason: "2FA code".into(),
                start_ms: Millis(1_100),
            },
            crate::ipc::ControlState::Takeover {
                start_ms: Millis(5_100),
                shortcuts: crate::ipc::HostShortcuts::Captured,
            },
        ]);
        journal.takeovers.push(crate::session_journal::Takeover {
            start: Millis(5_000),
            end: Some(Millis(8_000)),
        });

        let cards =
            crate::title_card::title_cards(journal.recovery_timeline(Millis(20_000)).events());

        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].1,
            crate::title_card::TitleCard::Answered {
                waited: Millis(4000),
                in_control: Millis(3000),
                reason: "2FA code".into(),
            }
        );
    }

    #[test]
    fn a_help_request_open_when_the_daemon_stopped_is_cut_to_the_end_of_the_recording() {
        let journal = journal_after(&[crate::ipc::ControlState::HelpRequest {
            reason: "2FA code".into(),
            start_ms: Millis(1_100),
        }]);

        let timeline = journal.recovery_timeline(Millis(61_000));

        assert_eq!(
            crate::title_card::cut_spans(&timeline, Millis(61_000), Millis(1_500)),
            [crate::title_card::Span {
                start: Millis(1_000),
                end: Millis(61_000),
            }]
        );
    }
}
