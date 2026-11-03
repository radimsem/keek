use super::run::LoopState;
use super::*;

pub(super) struct ReadyClose {
    summary: String,
    verdict: crate::checklist::Verdict,
    videos: PathBuf,
}
impl Actor {
    pub(super) async fn prepare_close(
        &mut self,
        inbox: &mut Inbox,
        timeline: &mut SessionTimeline,
        last_action: &Option<LastAction>,
        zero: Instant,
        deadline: &mut tokio::time::Instant,
        close: CloseRequest,
    ) -> Option<(CloseRequest, ReadyClose)> {
        *deadline = tokio::time::Instant::now() + self.idle_limit;
        for request in inbox.take_queued_checks() {
            *deadline = tokio::time::Instant::now() + self.idle_limit;
            match request {
                ActorRequest::AddCheck { text, reply } => {
                    self.add_check(last_action, timeline, text, reply).await
                }
                ActorRequest::MarkCheck {
                    check,
                    outcome,
                    comment,
                    reply,
                } => {
                    self.mark_check(last_action, timeline, zero, check, outcome, comment, reply)
                        .await
                }
                ActorRequest::Call { .. } | ActorRequest::RequestHelp { .. } => {
                    inbox.deferred.push_back(request)
                }
            }
        }
        let ready = match self.checklist.verdict() {
            Ok(verdict) => crate::evidence::writable_videos_dir(self.runner.as_ref())
                .await
                .map(|videos| ReadyClose {
                    summary: self.credentials.mask(&close.summary),
                    verdict,
                    videos,
                })
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        match ready {
            Ok(ready) => Some((close, ready)),
            Err(message) => {
                self.closing.store(false, Ordering::SeqCst);
                let _ = close.reply.send(self.mask_reply(refusal(message)));
                None
            }
        }
    }
    pub(super) async fn close_when_idle(
        &mut self,
        state: &mut LoopState,
        inbox: &mut Inbox,
        close: CloseRequest,
    ) -> Option<(CloseRequest, ReadyClose)> {
        let permit = loop {
            match self.control.gate.enter().await {
                gate::Entry::Run(permit) => break permit,
                gate::Entry::HandBack(_) => {}
            }
        };
        let ready = self
            .prepare_close(
                inbox,
                &mut state.timeline,
                &state.last_action,
                state.zero,
                &mut state.deadline,
                close,
            )
            .await;
        drop(permit);
        ready
    }
    pub(super) async fn finish_close(
        self,
        state: &mut LoopState,
        inbox: &mut Inbox,
        controller_task: &mut Option<tokio::task::JoinHandle<()>>,
        close: CloseRequest,
        ready: ReadyClose,
    ) -> Finished {
        let response = self
            .end_session(
                state.driver.as_mut(),
                inbox,
                controller_task,
                &mut state.timeline,
                state.zero,
                &state.last_action,
                Some(crate::checklist::SessionEnd::Verdict(ready.verdict)),
                Ok(ready.videos),
                ready.summary,
            )
            .await;
        Finished {
            response: self.mask_reply(response),
            name: self.name,
            reply: Some(close.reply),
        }
    }
}
