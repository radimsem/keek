use crate::{
    ipc::{SessionSnapshot, SessionState},
    session_folder::SessionName,
};
use tokio::sync::{mpsc, watch};

pub(super) fn watch(
    mut snapshots: watch::Receiver<SessionSnapshot>,
    name: SessionName,
    commands: mpsc::Sender<super::ViewerCommand>,
) -> gtk4::glib::JoinHandle<()> {
    gtk4::glib::spawn_future_local(async move {
        loop {
            if matches!(snapshots.borrow_and_update().state, SessionState::Ended(_)) {
                let _ = commands.send(super::ViewerCommand::Dismiss(name)).await;
                break;
            }
            if snapshots.changed().await.is_err() {
                break;
            }
        }
    })
}
