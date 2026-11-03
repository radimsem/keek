use crate::{ipc::SessionSnapshot, session_folder::SessionName};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
mod end_card;
mod events;
pub mod footer_view;
mod missing_card;
pub mod theme;
mod ui;
pub mod verdict_view;
pub use ui::{init, run};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId(pub usize);
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub name: SessionName,
    pub project: String,
    pub branch: Option<String>,
    pub goal: String,
    pub opened: chrono::DateTime<chrono::Utc>,
    pub slot: SlotId,
    pub placement: crate::slots::Slot,
    pub monitor: String,
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
    Done(SessionName),
    Dismiss(SessionName),
}
pub enum ViewerMessage {
    Commands(mpsc::Sender<ViewerCommand>),
    Opened(SessionInfo, watch::Receiver<SessionSnapshot>),
    Closed(SlotId),
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
            if let Err(error) = self.0.send(ViewerMessage::Opened(info, snapshots)).await {
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
