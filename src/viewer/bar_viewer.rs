use crate::{
    bar_state::{self, BAR_STATE_VERSION, BarSession, BarState},
    ipc::SessionSnapshot,
    viewer::{CardSpot, SessionInfo, SlotId, Still, Viewer, ViewerCommand, ViewerFuture},
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::{
    sync::{mpsc, watch},
    task::{AbortHandle, JoinHandle, JoinSet},
};

pub struct BarStateViewer {
    inner: Arc<dyn Viewer>,
    commands: mpsc::Sender<BarCommand>,
    writer: std::sync::Mutex<Option<JoinHandle<()>>>,
}

enum BarCommand {
    Opened(Box<SessionInfo>, watch::Receiver<SessionSnapshot>),
    Arranged(Vec<(SlotId, CardSpot)>),
    Closed(SlotId),
    Changed,
    Shutdown,
}

struct Card {
    info: SessionInfo,
    snapshots: watch::Receiver<SessionSnapshot>,
    in_focus: bool,
    forwarder: AbortHandle,
}

impl BarStateViewer {
    pub fn new(inner: Arc<dyn Viewer>, path: PathBuf) -> Arc<Self> {
        let (commands, receiver) = mpsc::channel(32);
        let writer = tokio::spawn(publish(path, receiver, commands.downgrade()));
        Arc::new(Self {
            inner,
            commands,
            writer: std::sync::Mutex::new(Some(writer)),
        })
    }

    /// Writes the last state, removes bar.json and ends the writer task, without shutting down the
    /// inner viewer. Its owner does that.
    pub async fn stop_writer(&self) {
        self.send(BarCommand::Shutdown).await;
        let writer = self.writer.lock().ok().and_then(|mut writer| writer.take());
        if let Some(writer) = writer
            && let Err(error) = writer.await
        {
            tracing::warn!(%error, "bar state writer stopped unexpectedly");
        }
    }

    async fn send(&self, command: BarCommand) {
        if let Err(error) = self.commands.send(command).await {
            tracing::warn!(%error, "bar state update could not be delivered");
        }
    }
}

impl Viewer for BarStateViewer {
    fn supports_cards(&self) -> bool {
        self.inner.supports_cards()
    }
    fn take_commands(&self) -> Option<mpsc::Receiver<ViewerCommand>> {
        self.inner.take_commands()
    }
    fn opened(
        &self,
        info: SessionInfo,
        snapshots: watch::Receiver<SessionSnapshot>,
    ) -> ViewerFuture<'_> {
        Box::pin(async move {
            self.inner.opened(info.clone(), snapshots.clone()).await;
            self.send(BarCommand::Opened(Box::new(info), snapshots))
                .await;
        })
    }
    fn closed(&self, slot: SlotId) -> ViewerFuture<'_> {
        Box::pin(async move {
            self.inner.closed(slot).await;
            self.send(BarCommand::Closed(slot)).await;
        })
    }
    fn arranged(&self, cards: Vec<(SlotId, CardSpot)>) -> ViewerFuture<'_> {
        Box::pin(async move {
            self.inner.arranged(cards.clone()).await;
            self.send(BarCommand::Arranged(cards)).await;
        })
    }
    fn still(&self, slot: SlotId, still: Still) -> ViewerFuture<'_> {
        self.inner.still(slot, still)
    }
    fn shutdown(&self) -> ViewerFuture<'_> {
        Box::pin(async move {
            self.inner.shutdown().await;
            self.stop_writer().await;
        })
    }
}

// The only task that writes bar.json, because every write reuses one temporary file.
async fn publish(
    path: PathBuf,
    mut commands: mpsc::Receiver<BarCommand>,
    sender: mpsc::WeakSender<BarCommand>,
) {
    let mut cards = BTreeMap::<SlotId, Card>::new();
    let mut forwarders = JoinSet::new();
    while let Some(command) = commands.recv().await {
        match command {
            BarCommand::Opened(info, snapshots) => {
                let forwarder = forwarders.spawn(forward(snapshots.clone(), sender.clone()));
                let card = Card {
                    in_focus: matches!(info.spot, CardSpot::Focus(_)),
                    info: *info,
                    snapshots,
                    forwarder,
                };
                if let Some(replaced) = cards.insert(card.info.slot, card) {
                    replaced.forwarder.abort();
                }
            }
            BarCommand::Arranged(spots) => {
                for (slot, spot) in spots {
                    if let Some(card) = cards.get_mut(&slot) {
                        card.in_focus = matches!(spot, CardSpot::Focus(_));
                    }
                }
            }
            BarCommand::Closed(slot) => {
                if let Some(closed) = cards.remove(&slot) {
                    closed.forwarder.abort();
                }
            }
            BarCommand::Changed => {}
            BarCommand::Shutdown => break,
        }
        while forwarders.try_join_next().is_some() {}
        let sessions = bar_sessions(&cards);
        store(path.clone(), sessions).await;
    }
    store(path, Vec::new()).await;
}

// Holds a weak sender so a dropped BarStateViewer closes the channel and ends the writer.
async fn forward(
    mut snapshots: watch::Receiver<SessionSnapshot>,
    sender: mpsc::WeakSender<BarCommand>,
) {
    while snapshots.changed().await.is_ok() {
        let Some(sender) = sender.upgrade() else {
            return;
        };
        if sender.send(BarCommand::Changed).await.is_err() {
            return;
        }
    }
}

fn bar_sessions(cards: &BTreeMap<SlotId, Card>) -> Vec<BarSession> {
    let now = chrono::Utc::now();
    cards
        .values()
        .map(|card| {
            bar_state::bar_session(&card.info, &card.snapshots.borrow(), card.in_focus, now)
        })
        .collect()
}

async fn store(path: PathBuf, sessions: Vec<BarSession>) {
    let stored = tokio::task::spawn_blocking(move || {
        if sessions.is_empty() {
            bar_state::remove(&path)
        } else {
            bar_state::write(
                &path,
                &BarState {
                    version: BAR_STATE_VERSION,
                    sessions,
                },
            )
        }
    })
    .await;
    match stored {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "bar state could not be updated"),
        Err(error) => tracing::warn!(%error, "bar state write did not finish"),
    }
}

#[cfg(test)]
#[path = "bar_viewer_tests.rs"]
mod tests;
