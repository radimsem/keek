use crate::{
    ipc::Response,
    session_folder::SessionName,
    timeline::{CheckId, CheckOutcome},
};
use serde_json::Value;
use tokio::sync::oneshot;
impl ActorRequest {
    pub fn into_reply(self) -> oneshot::Sender<Response> {
        match self {
            Self::Call { reply, .. }
            | Self::RequestHelp { reply, .. }
            | Self::AddCheck { reply, .. }
            | Self::MarkCheck { reply, .. }
            | Self::Close { reply, .. } => reply,
        }
    }
}
pub enum ActorRequest {
    RequestHelp {
        arrival: crate::session::gate::Arrival,
        reason: String,
        reply: oneshot::Sender<Response>,
    },
    Call {
        arrival: crate::session::gate::Arrival,
        tool: String,
        arguments: Value,
        reply: oneshot::Sender<Response>,
    },
    AddCheck {
        text: String,
        reply: oneshot::Sender<Response>,
    },
    MarkCheck {
        check: CheckId,
        outcome: CheckOutcome,
        comment: String,
        reply: oneshot::Sender<Response>,
    },
    Close {
        summary: String,
        reply: oneshot::Sender<Response>,
    },
}
pub struct Finished {
    pub name: SessionName,
    pub response: Response,
    pub reply: Option<oneshot::Sender<Response>>,
}
