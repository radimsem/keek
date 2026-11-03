use super::*;

async fn open_for(h: &mut Harness, alive: &watch::Sender<()>, path: PathBuf) -> SessionName {
    let (reply, answer) = oneshot::channel();
    h.registry
        .handle_connection(
            Some(alive.subscribe()),
            Request::OpenSession {
                project: "app".into(),
                worktree: path,
                goal: "goal".into(),
                checks: vec!["check".into()],
            },
            reply,
            None,
        )
        .await;
    let prepared = h.registry.preparing.join_next().await.unwrap().unwrap();
    h.registry.prepared(prepared).await;
    let Response::Opened { session, .. } = answer.await.unwrap() else {
        panic!("open failed")
    };
    session
}

#[tokio::test]
async fn a_disconnect_ends_only_the_sessions_that_connection_opened() {
    let mut h = Harness::new(3);
    let (first, _) = watch::channel(());
    let (other, _) = watch::channel(());
    let path = h.worktree("first");
    let a = open_for(&mut h, &first, path).await;
    let path = h.worktree("second");
    let b = open_for(&mut h, &first, path).await;
    let path = h.worktree("other");
    let c = open_for(&mut h, &other, path).await;
    drop(first);
    for _ in 0..2 {
        let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
        assert!(finished.name == a || finished.name == b);
        let Response::Closed(value) = &finished.response else {
            panic!("{:?}", finished.response)
        };
        assert!(
            value["evidence"]["report"]
                .as_str()
                .unwrap()
                .contains("incomplete")
        );
        h.registry.finished(finished).await;
    }
    assert_eq!(h.registry.len(), 1);
    assert!(matches!(
        h.request(Request::CallTool {
            session: c,
            tool: "click".into(),
            arguments: json!({})
        })
        .await,
        Response::ToolResult(_)
    ));
}

#[tokio::test]
async fn failed_incomplete_filing_keeps_the_journal_for_recovery() {
    let mut h = Harness::new(1);
    let (alive, _) = watch::channel(());
    let path = h.worktree("filing-failure");
    let name = open_for(&mut h, &alive, path).await;
    std::fs::write(h.directory.path().join("videos"), "not a directory").unwrap();
    drop(alive);
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    assert!(matches!(finished.response, Response::Refused { .. }));
    assert!(
        h.directory
            .path()
            .join(format!("state/sessions/{name}.json"))
            .exists()
    );
    h.registry.finished(finished).await;
}

#[tokio::test]
async fn disconnect_during_install_drops_the_installer_and_frees_the_slot() {
    struct Installer(Arc<tokio::sync::Notify>, Arc<AtomicBool>);
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    impl DriverFactory for Installer {
        fn prepare<'a>(
            &'a self,
            _: &'a SessionFolder,
            _: &'a crate::session::Opening,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<crate::session::Mounts, SessionError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let _flag = DropFlag(self.1.clone());
                self.0.notify_one();
                std::future::pending().await
            })
        }
        fn open<'a>(
            &'a self,
            _: &'a SessionFolder,
            _: &'a crate::session::Opening,
            _: &'a crate::session::Mounts,
        ) -> crate::session::OpenDriverFuture<'a> {
            Box::pin(async { panic!("install never finished") })
        }
    }
    let mut h = Harness::new(1);
    let entered = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    h.registry.dependencies.factory = Arc::new(Installer(entered.clone(), dropped.clone()));
    let (alive, opener) = watch::channel(());
    let path = h.worktree("install");
    let (reply, _answer) = oneshot::channel();
    h.registry
        .handle_connection(
            Some(opener),
            Request::OpenSession {
                project: "app".into(),
                worktree: path,
                goal: "goal".into(),
                checks: vec![],
            },
            reply,
            None,
        )
        .await;
    let prepared = h.registry.preparing.join_next().await.unwrap().unwrap();
    h.registry.prepared(prepared).await;
    entered.notified().await;
    drop(alive);
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    let folder = h
        .registry
        .dependencies
        .sessions_dir
        .join(finished.name.to_string());
    h.registry.finished(finished).await;
    assert!(dropped.load(Ordering::SeqCst));
    assert!(!folder.exists());
    assert!(h.registry.is_empty());
    assert!(!h.directory.path().join("videos").exists());
}

#[tokio::test]
async fn disconnect_interrupts_a_tool_call_in_progress() {
    let mut h = Harness::new(1);
    let (alive, _) = watch::channel(());
    let path = h.worktree("slow");
    let name = open_for(&mut h, &alive, path).await;
    let (reply, answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CallTool {
                session: name,
                tool: "slow".into(),
                arguments: json!({}),
            },
            reply,
            None,
        )
        .await;
    h.controls.started.notified().await;
    drop(alive);
    let Response::Refused { message } = answer.await.unwrap() else {
        panic!("call was not refused")
    };
    assert_eq!(
        message,
        "The coding agent disconnected, so keek ended the session and filed it as an Incomplete Session. Open a new Agent Session."
    );
    let finished = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        h.registry.tasks.join_next(),
    )
    .await
    .expect("disconnect left the tool call running")
    .unwrap()
    .unwrap();
    assert!(matches!(finished.response, Response::Closed(_)));
}

#[tokio::test]
async fn disconnect_leaves_a_session_already_closing_alone() {
    let mut h = Harness::new(1);
    let (alive, _) = watch::channel(());
    let path = h.worktree("closing");
    let name = open_for(&mut h, &alive, path).await;
    h.mark(&name).await;
    h.controls.stop_hangs.store(true, Ordering::SeqCst);
    let (reply, answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: name,
                summary: "done".into(),
            },
            reply,
            None,
        )
        .await;
    h.controls.started.notified().await;
    drop(alive);
    h.controls.release.notify_one();
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    h.registry.finished(finished).await;
    let Response::Closed(value) = answer.await.unwrap() else {
        panic!("close refused")
    };
    assert_eq!(value["verdict"], "Pass");
}

#[tokio::test]
async fn a_queued_close_wins_over_disconnect_during_a_call() {
    let mut h = Harness::new(1);
    let (alive, _) = watch::channel(());
    let path = h.worktree("queued-close");
    let name = open_for(&mut h, &alive, path).await;
    h.mark(&name).await;
    let (reply, call) = oneshot::channel();
    h.registry
        .handle(
            Request::CallTool {
                session: name.clone(),
                tool: "slow".into(),
                arguments: json!({}),
            },
            reply,
            None,
        )
        .await;
    h.controls.started.notified().await;
    let (reply, close) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: name,
                summary: "done".into(),
            },
            reply,
            None,
        )
        .await;
    drop(alive);
    tokio::task::yield_now().await;
    h.controls.release.notify_one();
    assert!(matches!(call.await.unwrap(), Response::ToolResult(_)));
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    h.registry.finished(finished).await;
    let Response::Closed(value) = close.await.unwrap() else {
        panic!("close was not filed")
    };
    assert_eq!(value["verdict"], "Pass");
}

#[tokio::test(start_paused = true)]
async fn a_queued_close_takes_precedence_over_an_expired_idle_deadline() {
    let mut h = Harness::new(1);
    h.registry.dependencies.session_idle_limit = std::time::Duration::from_secs(3);
    let (alive, _) = watch::channel(());
    let path = h.worktree("closing");
    let name = open_for(&mut h, &alive, path).await;
    let (reply, marked) = oneshot::channel();
    h.registry
        .handle(
            Request::MarkCheck {
                session: name.clone(),
                check: crate::timeline::CheckId(1),
                outcome: crate::timeline::CheckOutcome::Passed,
                comment: String::new(),
            },
            reply,
            None,
        )
        .await;
    // The actor takes the mark, restarts the idle limit and waits on its journal write.
    tokio::task::yield_now().await;
    let (reply, answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: name,
                summary: "done".into(),
            },
            reply,
            None,
        )
        .await;
    // The limit runs out before the actor is back in its select, with the close already queued.
    tokio::time::advance(std::time::Duration::from_secs(4)).await;
    assert!(matches!(
        marked.await.unwrap(),
        Response::CheckMarked { .. }
    ));
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    h.registry.finished(finished).await;
    let response = answer.await.unwrap();
    assert!(
        matches!(response, Response::Closed(ref value) if value["verdict"] == "Pass"),
        "{response:?}"
    );
}
