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
        let changed = match state {
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
                let end = Millis(
                    (clock.now().saturating_duration_since(opened).as_millis() as u64)
                        .saturating_sub(offset),
                );
                if let Some(span) = data.takeovers.last_mut()
                    && span.end.is_none()
                {
                    span.end = Some(end);
                    true
                } else {
                    false
                }
            }
        };
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
            credentials_hash: None,
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
}
