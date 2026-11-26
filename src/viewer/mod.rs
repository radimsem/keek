use crate::{ipc::SessionSnapshot, session_folder::SessionName};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
pub mod bar_viewer;
mod end_card;
mod events;
mod focus_card;
mod footer_actions;
mod footer_content;
pub mod footer_view;
mod missing_card;
mod paused_card;
mod rail_card;
mod rail_panels;
mod rail_style;
pub mod rail_view;
mod style;
pub mod theme;
mod ui;
pub mod verdict_view;
pub use events::show_target;
pub use ui::{init, run};
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardSpot {
    Focus(crate::slots::CardGeometry),
    Rail(crate::slots::Slot),
}
#[derive(Debug, Clone)]
pub struct Still {
    pub path: std::path::PathBuf,
    pub taken: chrono::DateTime<chrono::Utc>,
}
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub name: SessionName,
    pub project: String,
    pub branch: Option<String>,
    pub goal: String,
    pub opened: chrono::DateTime<chrono::Utc>,
    pub slot: SlotId,
    pub placement: crate::slots::CardGeometry,
    pub monitor: String,
    pub spot: CardSpot,
}
pub type ViewerFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
pub trait Viewer: Send + Sync {
    fn supports_cards(&self) -> bool {
        true
    }
    fn take_commands(&self) -> Option<mpsc::Receiver<ViewerCommand>> {
        None
    }
    fn opened(
        &self,
        info: SessionInfo,
        snapshots: watch::Receiver<SessionSnapshot>,
    ) -> ViewerFuture<'_>;
    fn closed(&self, slot: SlotId) -> ViewerFuture<'_>;
    fn arranged(&self, _: Vec<(SlotId, CardSpot)>) -> ViewerFuture<'_> {
        Box::pin(async {})
    }
    fn still(&self, _: SlotId, _: Still) -> ViewerFuture<'_> {
        Box::pin(async {})
    }
    fn shutdown(&self) -> ViewerFuture<'_> {
        Box::pin(async {})
    }
}
pub struct NoopViewer;
impl Viewer for NoopViewer {
    fn supports_cards(&self) -> bool {
        false
    }
    fn opened(&self, _: SessionInfo, _: watch::Receiver<SessionSnapshot>) -> ViewerFuture<'_> {
        Box::pin(async {})
    }
    fn closed(&self, _: SlotId) -> ViewerFuture<'_> {
        Box::pin(async {})
    }
}
pub enum ViewerCommand {
    TakeControl(SessionName),
    StepAway(SessionName),
    Resume(SessionName),
    Done(SessionName),
    Dismiss(SessionName),
    Show(ShowTarget),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShowTarget {
    Session(SessionName),
    Number(usize),
}
pub enum ViewerMessage {
    Commands(mpsc::Sender<ViewerCommand>),
    Opened(Box<SessionInfo>, watch::Receiver<SessionSnapshot>),
    Closed(SlotId),
    Arranged(Vec<(SlotId, CardSpot)>),
    Still(SlotId, Still),
    Shutdown,
}
struct ChannelViewer(
    mpsc::Sender<ViewerMessage>,
    std::sync::Mutex<Option<mpsc::Receiver<ViewerCommand>>>,
);
impl Viewer for ChannelViewer {
    fn take_commands(&self) -> Option<mpsc::Receiver<ViewerCommand>> {
        self.1.lock().ok().and_then(|mut receiver| receiver.take())
    }
    fn shutdown(&self) -> ViewerFuture<'_> {
        Box::pin(async {
            let _ = self.0.send(ViewerMessage::Shutdown).await;
        })
    }
    fn opened(
        &self,
        info: SessionInfo,
        snapshots: watch::Receiver<SessionSnapshot>,
    ) -> ViewerFuture<'_> {
        Box::pin(async move {
            if let Err(error) = self
                .0
                .send(ViewerMessage::Opened(Box::new(info), snapshots))
                .await
            {
                tracing::warn!(%error,"Session Footer update could not be delivered");
            }
        })
    }
    fn closed(&self, slot: SlotId) -> ViewerFuture<'_> {
        Box::pin(async move {
            if let Err(error) = self.0.send(ViewerMessage::Closed(slot)).await {
                tracing::warn!(%error,"Session Footer close could not be delivered");
            }
        })
    }
    fn arranged(&self, cards: Vec<(SlotId, CardSpot)>) -> ViewerFuture<'_> {
        Box::pin(async move {
            if let Err(error) = self.0.send(ViewerMessage::Arranged(cards)).await {
                tracing::warn!(%error, "Session Card arrangement could not be delivered");
            }
        })
    }
    fn still(&self, slot: SlotId, still: Still) -> ViewerFuture<'_> {
        Box::pin(async move {
            if let Err(error) = self.0.send(ViewerMessage::Still(slot, still)).await {
                tracing::warn!(%error, "rail still could not be delivered");
            }
        })
    }
}
pub fn channel() -> (Arc<dyn Viewer>, mpsc::Receiver<ViewerMessage>) {
    let (tx, rx) = mpsc::channel(32);
    let (commands, command_receiver) = mpsc::channel(32);
    let _ = tx.try_send(ViewerMessage::Commands(commands));
    (
        Arc::new(ChannelViewer(
            tx,
            std::sync::Mutex::new(Some(command_receiver)),
        )),
        rx,
    )
}
