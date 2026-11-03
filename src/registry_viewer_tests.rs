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

#[tokio::test]
async fn an_ended_card_keeps_the_daemon_alive_without_counting_as_a_session() {
    let mut h = Harness::new(1);
    let viewer = Arc::new(ViewerEvents::default());
    h.registry.dependencies.viewer = viewer.clone();
    let name = h.open(h.worktree("one")).await;
    h.finish(name).await;
    assert_eq!(h.registry.len(), 0);
    assert!(h.registry.keeps_daemon_alive());
    assert_eq!(
        *viewer.0.lock().unwrap(),
        vec![(crate::viewer::SlotId(0), "app".into())]
    );
}

#[tokio::test]
async fn shutdown_succeeds_when_only_an_ended_card_remains() {
    let mut h = Harness::new(1);
    h.registry.dependencies.viewer = Arc::new(ViewerEvents::default());
    let name = h.open(h.worktree("shutdown-card")).await;
    h.finish(name).await;
    assert!(matches!(h.registry.shutdown(), Response::ShuttingDown));
}

#[tokio::test]
async fn dismiss_before_the_finished_actor_is_processed_does_not_leave_a_card() {
    let mut h = Harness::new(1);
    h.registry.dependencies.viewer = Arc::new(ViewerEvents::default());
    let name = h.open(h.worktree("missing-monitor")).await;
    std::fs::write(
        h.registry
            .dependencies
            .sessions_dir
            .join(name.to_string())
            .join("recording.mp4"),
        b"recording",
    )
    .unwrap();
    h.mark(&name).await;
    let (reply, _answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: name.clone(),
                summary: "done".into(),
            },
            reply,
            None,
        )
        .await;
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    let state = h
        .registry
        .sessions
        .get(&name)
        .unwrap()
        .events
        .borrow()
        .state
        .clone();
    let SessionState::Ended(end) = state else {
        panic!("missing ended state")
    };
    let frame = end.frame.unwrap();
    std::fs::write(&frame, b"frame").unwrap();
    h.registry
        .control(crate::viewer::ViewerCommand::Dismiss(name.clone()))
        .await;
    h.registry.finished(finished).await;
    assert!(!h.registry.cards.contains_key(&name));
    assert!(!frame.exists());
    assert!(!h.registry.keeps_daemon_alive());
}

#[tokio::test]
async fn dismissing_the_last_card_frees_its_slot_and_lets_the_daemon_go_idle() {
    let mut h = Harness::new(1);
    let viewer = Arc::new(ViewerEvents::default());
    h.registry.dependencies.viewer = viewer.clone();
    let name = h.open(h.worktree("one")).await;
    h.finish(name.clone()).await;
    assert!(h.registry.keeps_daemon_alive());
    h.registry
        .control(crate::viewer::ViewerCommand::Dismiss(name))
        .await;
    assert!(!h.registry.keeps_daemon_alive());
    assert_eq!(
        viewer.0.lock().unwrap().last(),
        Some(&(crate::viewer::SlotId(0), "closed".into()))
    );
    let next = h.open(h.worktree("two")).await;
    assert_eq!(
        viewer.0.lock().unwrap().last(),
        Some(&(crate::viewer::SlotId(0), "app".into()))
    );
    h.finish(next).await;
}

#[tokio::test]
async fn a_new_session_replaces_the_oldest_card_when_every_slot_holds_a_card() {
    let mut h = Harness::new(2);
    let viewer = Arc::new(ViewerEvents::default());
    h.registry.dependencies.viewer = viewer.clone();
    let first = h.open(h.worktree("one")).await;
    let second = h.open(h.worktree("two")).await;
    h.finish(second.clone()).await;
    h.finish(first).await;
    let frame = h.directory.path().join("replaced-frame.png");
    std::fs::write(&frame, b"frame").unwrap();
    h.registry.cards.get_mut(&second).unwrap().frame = Some(frame.clone());
    assert_eq!(viewer.0.lock().unwrap().len(), 2);
    let next = h.open(h.worktree("three")).await;
    assert!(!frame.exists());
    assert_eq!(
        &viewer.0.lock().unwrap()[2..],
        &[
            (crate::viewer::SlotId(1), "closed".into()),
            (crate::viewer::SlotId(1), "app".into())
        ]
    );
    h.finish(next).await;
}

#[tokio::test]
async fn a_slot_failure_keeps_the_old_card_and_frame() {
    struct NoSlot;
    impl crate::compositor::Compositor for NoSlot {
        fn window_events(&self) -> crate::compositor::EventsFuture<'_> {
            Box::pin(async { Ok(None) })
        }
        fn place_window(
            &self,
            _: Option<crate::compositor::WindowPid>,
            _: crate::compositor::Placement,
            _: Option<crate::compositor::WindowEvents>,
        ) -> crate::compositor::WindowFuture<'_> {
            Box::pin(async {
                Err(ProcessError::Failed {
                    program: "fixture".into(),
                    message: "no slot".into(),
                })
            })
        }
        fn slot(
            &self,
            _: crate::viewer::SlotId,
            _: usize,
            _: Option<crate::project::WindowSize>,
        ) -> crate::compositor::SlotFuture<'_> {
            Box::pin(async {
                Err(ProcessError::Failed {
                    program: "fixture".into(),
                    message: "no slot".into(),
                })
            })
        }
        fn install(&self) -> CompositorFuture<'_> {
            Box::pin(async { Ok(()) })
        }
        fn remove(&self) -> CompositorFuture<'_> {
            Box::pin(async { Ok(()) })
        }
    }
    let mut h = Harness::new(1);
    let viewer = Arc::new(ViewerEvents::default());
    h.registry.dependencies.viewer = viewer.clone();
    let name = h.open(h.worktree("old-card")).await;
    h.finish(name.clone()).await;
    let frame = h.directory.path().join("frame.png");
    std::fs::write(&frame, b"frame").unwrap();
    h.registry.cards.get_mut(&name).unwrap().frame = Some(frame.clone());
    h.registry.dependencies.compositor = Arc::new(NoSlot);
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree: h.worktree("no-slot"),
            goal: "goal".into(),
            checks: vec![],
        })
        .await;
    assert!(matches!(response, Response::Refused { .. }));
    assert!(h.registry.cards.contains_key(&name));
    assert!(frame.exists());
    assert_eq!(viewer.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_noop_viewer_frees_the_slot_as_soon_as_the_session_ends() {
    let mut h = Harness::new(1);
    let first = h.open(h.worktree("one")).await;
    h.finish(first).await;
    assert!(!h.registry.keeps_daemon_alive());
    let next = h.open(h.worktree("two")).await;
    assert_eq!(
        h.registry.sessions.get(&next).unwrap().slot,
        crate::viewer::SlotId(0)
    );
    h.finish(next).await;
}

#[tokio::test]
async fn dismiss_removes_the_frame_before_returning() {
    let mut h = Harness::new(1);
    let viewer = Arc::new(ViewerEvents::default());
    h.registry.dependencies.viewer = viewer.clone();
    let name = h.open(h.worktree("one")).await;
    h.finish(name.clone()).await;
    let frame = h.directory.path().join("last-frame.png");
    std::fs::write(&frame, b"fixture").unwrap();
    h.registry.cards.get_mut(&name).unwrap().frame = Some(frame.clone());
    h.registry
        .control(crate::viewer::ViewerCommand::Dismiss(name))
        .await;
    assert!(!frame.exists());
    assert_eq!(
        viewer.0.lock().unwrap().last(),
        Some(&(crate::viewer::SlotId(0), "closed".into()))
    );
}

#[tokio::test]
async fn a_noop_viewer_removes_the_captured_frame() {
    struct CaptureRunner {
        inner: Runner,
        captured: Arc<std::sync::atomic::AtomicBool>,
    }
    impl ProcessRunner for CaptureRunner {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            cwd: Option<&'a Path>,
        ) -> crate::process::ProcessFuture<'a> {
            Box::pin(async move {
                if program == "ffmpeg" && args.contains(&"-sseof") {
                    std::fs::write(args.last().unwrap(), b"frame").unwrap();
                    self.captured.store(true, Ordering::SeqCst);
                    return Ok(String::new());
                }
                self.inner.output(program, args, cwd).await
            })
        }
    }
    let mut h = Harness::new(1);
    let captured = Arc::new(std::sync::atomic::AtomicBool::new(false));
    h.registry.dependencies.runner = Arc::new(CaptureRunner {
        inner: Runner(h.directory.path().join("videos")),
        captured: captured.clone(),
    });
    let name = h.open(h.worktree("noop-frame")).await;
    std::fs::write(
        h.registry
            .dependencies
            .sessions_dir
            .join(name.to_string())
            .join("recording.mp4"),
        b"recording",
    )
    .unwrap();
    let frame = h
        .registry
        .dependencies
        .frames_dir
        .join(format!("{name}.png"));
    h.finish(name).await;
    assert!(captured.load(Ordering::SeqCst));
    assert!(!frame.exists());
}

#[tokio::test]
async fn rendering_snapshots_are_published_while_the_editor_runs() {
    struct ProgressRunner {
        inner: Runner,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        blocked: std::sync::atomic::AtomicBool,
    }
    impl ProcessRunner for ProgressRunner {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            cwd: Option<&'a Path>,
        ) -> crate::process::ProcessFuture<'a> {
            if program == "ffprobe" {
                return Box::pin(async { Ok("10.0".into()) });
            }
            if program == "fc-match" {
                return Box::pin(async { Ok("JetBrains Mono|/fonts/mono.ttf".into()) });
            }
            self.inner.output(program, args, cwd)
        }
        fn stream<'a>(
            &'a self,
            _: &'a str,
            args: &'a [&'a str],
            progress: &'a mut (dyn FnMut(&str) + Send),
        ) -> crate::process::ProcessFuture<'a> {
            Box::pin(async move {
                if !args.contains(&"null") {
                    progress("out_time_us=1000000");
                    if !self.blocked.swap(true, Ordering::SeqCst) {
                        self.entered.notify_one();
                        self.release.notified().await;
                    }
                    std::fs::write(args.last().unwrap(), b"video").unwrap();
                }
                Ok(String::new())
            })
        }
    }
    let mut h = Harness::new(1);
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    h.registry.dependencies.runner = Arc::new(ProgressRunner {
        inner: Runner(h.directory.path().join("videos")),
        entered: entered.clone(),
        release: release.clone(),
        blocked: std::sync::atomic::AtomicBool::new(false),
    });
    let name = h.open(h.worktree("progress")).await;
    std::fs::write(
        h.registry
            .dependencies
            .sessions_dir
            .join(name.to_string())
            .join("recording.mp4"),
        b"raw",
    )
    .unwrap();
    h.mark(&name).await;
    let events = h.registry.sessions.get(&name).unwrap().events.clone();
    let (reply, answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: name,
                summary: String::new(),
            },
            reply,
            None,
        )
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    assert!(
        matches!(events.borrow().state, SessionState::Rendering { percent: crate::ipc::Percent(value), .. } if value > 0)
    );
    release.notify_one();
    let finished = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        h.registry.tasks.join_next(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    h.registry.finished(finished).await;
    assert!(matches!(answer.await.unwrap(), Response::Closed(_)));
}
