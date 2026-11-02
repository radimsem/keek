use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct ViewerEvents(Mutex<Vec<(crate::viewer::SlotId, String)>>);
impl crate::viewer::Viewer for ViewerEvents {
    fn opened(
        &self,
        info: crate::viewer::SessionInfo,
        _: watch::Receiver<SessionSnapshot>,
    ) -> crate::viewer::ViewerFuture<'_> {
        Box::pin(async move {
            self.0.lock().unwrap().push((info.slot, info.project));
        })
    }
    fn closed(&self, slot: crate::viewer::SlotId) -> crate::viewer::ViewerFuture<'_> {
        Box::pin(async move {
            self.0.lock().unwrap().push((slot, "closed".into()));
        })
    }
}
#[tokio::test]
async fn the_viewer_gets_session_info_and_reuses_the_freed_slot() {
    let mut h = Harness::new(2);
    let viewer = Arc::new(ViewerEvents::default());
    h.registry.dependencies.viewer = viewer.clone();
    let first = h.open(h.worktree("one")).await;
    let second = h.open(h.worktree("two")).await;
    h.finish(first).await;
    let third = h.open(h.worktree("three")).await;
    assert_eq!(
        *viewer.0.lock().unwrap(),
        vec![
            (crate::viewer::SlotId(0), "app".into()),
            (crate::viewer::SlotId(1), "app".into()),
            (crate::viewer::SlotId(0), "closed".into()),
            (crate::viewer::SlotId(0), "app".into())
        ]
    );
    h.finish(second).await;
    h.finish(third).await;
}

#[tokio::test]
async fn a_failed_open_closes_the_card_and_frees_the_slot() {
    let mut h = Harness::new(2);
    let viewer = Arc::new(ViewerEvents::default());
    h.registry.dependencies.viewer = viewer.clone();
    h.controls
        .fail_open
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let failed = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree: h.worktree("failed"),
            goal: "goal".into(),
            checks: vec![],
        })
        .await;
    assert!(matches!(failed, Response::Refused { .. }));
    let session = h.open(h.worktree("next")).await;
    assert_eq!(
        *viewer.0.lock().unwrap(),
        vec![
            (crate::viewer::SlotId(0), "app".into()),
            (crate::viewer::SlotId(0), "closed".into()),
            (crate::viewer::SlotId(0), "app".into()),
        ]
    );
    h.finish(session).await;
}
