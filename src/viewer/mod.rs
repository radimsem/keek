use crate::{ipc::SessionSnapshot, session_folder::SessionName};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
mod events;
pub mod footer_view;
pub mod theme;
mod ui;
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
    fn opened(&self, _: SessionInfo, _: watch::Receiver<SessionSnapshot>) -> ViewerFuture<'_> {
        Box::pin(async {})
    }
    fn closed(&self, _: SlotId) -> ViewerFuture<'_> {
        Box::pin(async {})
    }
}
pub enum ViewerMessage {
    Opened(SessionInfo, watch::Receiver<SessionSnapshot>),
    Closed(SlotId),
    Shutdown,
}
struct ChannelViewer(mpsc::Sender<ViewerMessage>);
impl Viewer for ChannelViewer {
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
    (Arc::new(ChannelViewer(tx)), rx)
}
