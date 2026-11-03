use super::*;
use std::collections::VecDeque;

pub struct Inbox {
    pub(super) requests: mpsc::Receiver<ActorRequest>,
    pub(super) close: mpsc::Receiver<CloseRequest>,
    pub(super) deferred: VecDeque<ActorRequest>,
}
impl Inbox {
    pub fn new(
        requests: mpsc::Receiver<ActorRequest>,
        close: mpsc::Receiver<CloseRequest>,
    ) -> Self {
        Self {
            requests,
            close,
            deferred: VecDeque::new(),
        }
    }
    pub(super) async fn next_request(
        requests: &mut mpsc::Receiver<ActorRequest>,
        deferred: &mut VecDeque<ActorRequest>,
    ) -> Option<ActorRequest> {
        if let Some(request) = deferred.pop_front() {
            return Some(request);
        }
        requests.recv().await
    }
    pub(super) async fn next_close(
        close: &mut mpsc::Receiver<CloseRequest>,
    ) -> Option<CloseRequest> {
        match close.recv().await {
            Some(close) => Some(close),
            None => std::future::pending().await,
        }
    }
    pub(super) fn take_queued_checks(&mut self) -> Vec<ActorRequest> {
        let mut checks = Vec::new();
        while let Ok(request) = self.requests.try_recv() {
            match request {
                ActorRequest::AddCheck { .. } | ActorRequest::MarkCheck { .. } => {
                    checks.push(request)
                }
                ActorRequest::Call { .. } | ActorRequest::RequestHelp { .. } => {
                    self.deferred.push_back(request)
                }
            }
        }
        checks
    }
    pub(super) fn refuse_all(&mut self, name: &SessionName) {
        self.requests.close();
        self.close.close();
        let message = format!("Agent Session {name} is closing. Open a new Agent Session.");
        for request in self.deferred.drain(..) {
            let _ = request.into_reply().send(refusal(&message));
        }
        while let Ok(request) = self.requests.try_recv() {
            let _ = request.into_reply().send(refusal(&message));
        }
        while let Ok(request) = self.close.try_recv() {
            let _ = request.reply.send(refusal(&message));
        }
    }
}
