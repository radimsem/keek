use super::*;

#[tokio::test(start_paused = true)]
async fn project_prepare_can_outlast_the_desktop_handshake_limit() {
    struct LongPrepare;
    impl DriverFactory for LongPrepare {
        fn prepare<'a>(
            &'a self,
            _: &'a SessionFolder,
            _: &'a Opening,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Mounts, SessionError>> + Send + 'a>,
        > {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(Mounts::default())
            })
        }
        fn open<'a>(
            &'a self,
            _: &'a SessionFolder,
            _: &'a Opening,
            _: &'a Mounts,
        ) -> OpenDriverFuture<'a> {
            Box::pin(async {
                Err(SessionError::Refused(
                    "desktop reached after prepare".into(),
                ))
            })
        }
    }
    let root = tempfile::tempdir().unwrap();
    let name = "app_12-00-00".parse().unwrap();
    let folder = SessionFolder::new(root.path(), &name);
    let (events, _) = watch::channel(SessionSnapshot {
        control: crate::ipc::ControlState::Verifier,
        state: SessionState::Opening,
        checks: vec![],
        last_action: None,
    });
    let (control, control_commands) = control::Control::new();
    let actor = Actor {
        control,
        control_commands: Some(control_commands),
        compositor: Arc::new(FakeCompositor),
        opened_at: Instant::now(),
        placement: crate::compositor::Placement {
            monitor: "fake".into(),
            origin: (0, 0),
            slot: crate::slots::Slot {
                x: 24,
                y: 50,
                width: 1280,
                height: 800,
            },
        },
        credentials: Default::default(),
        project: "app".into(),
        worktree: root.path().to_owned(),
        settings: crate::project::Settings::default(),
        warnings: vec![],
        goal: "fixture".into(),
        revision: Revision {
            branch: None,
            commit: "fixture".into(),
            dirty: false,
        },
        started: chrono::Local::now(),
        checklist: Checklist::new(vec![]),
        runner: Arc::new(takeover::Runner {
            inner: EvidenceRunner(root.path().join("videos"), Arc::new(AtomicBool::new(false))),
            calls: Arc::new(Mutex::new(vec![])),
            fail_no_focus: Arc::new(AtomicBool::new(false)),
        }),
        closing: Arc::new(AtomicBool::new(false)),
        name,
        folder,
        factory: Arc::new(LongPrepare),
        clock: Arc::new(FakeClock {
            zero: Instant::now(),
            elapsed: AtomicU64::new(0),
        }),
        catalog: Arc::new(Catalog::default()),
        events,
    };
    let (_sender, mailbox) = mpsc::channel(1);
    let (reply, _response) = oneshot::channel();
    let start = tokio::time::Instant::now();
    let finished = actor.run(mailbox, reply).await;
    assert!(tokio::time::Instant::now() - start >= Duration::from_secs(60));
    assert_eq!(
        finished.response,
        Response::Refused {
            message: "desktop reached after prepare".into()
        }
    );
}
