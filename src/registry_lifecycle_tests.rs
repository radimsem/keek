use super::*;

#[tokio::test]
async fn unknown_session_says_when_none_are_open() {
    let mut h = Harness::new(2);
    let message = refused(
        h.request(Request::CallTool {
            session: "missing".parse().unwrap(),
            tool: "click".into(),
            arguments: json!({}),
        })
        .await,
    );
    assert!(message.contains("none"), "{message}");
}

#[tokio::test]
async fn a_relative_worktree_is_refused_by_the_daemon() {
    let mut h = Harness::new(2);
    let message = refused(
        h.request(Request::OpenSession {
            project: "app".into(),
            worktree: "repo".into(),
            goal: "goal".into(),
            checks: vec!["check".into()],
        })
        .await,
    );
    assert!(message.contains("absolute"), "{message}");
}

#[tokio::test]
async fn a_watch_reports_state_and_the_last_action_until_the_session_ends() {
    let mut h = Harness::new(2);
    let name = h.open(h.worktree("one")).await;
    let (events, mut received) = mpsc::channel(8);
    let (reply, ended) = oneshot::channel();
    h.registry
        .handle(
            Request::WatchSession {
                session: name.clone(),
            },
            reply,
            Some(events),
        )
        .await;
    assert!(matches!(
        received.recv().await,
        Some(Event::StateChanged {
            state: SessionState::Open
        })
    ));

    let called = h
        .request(Request::CallTool {
            session: name.clone(),
            tool: "click".into(),
            arguments: json!({}),
        })
        .await;
    assert!(matches!(called, Response::ToolResult(_)), "{called:?}");
    let action = received.recv().await;
    assert!(
        matches!(&action, Some(Event::LastAction { tool, .. }) if tool == "click"),
        "{action:?}"
    );

    assert!(matches!(h.finish(name).await, Response::Closed(_)));
    assert!(matches!(
        received.recv().await,
        Some(Event::StateChanged {
            state: SessionState::Closing
        })
    ));
    assert!(matches!(ended.await.unwrap(), Response::WatchEnded));
}

#[tokio::test]
async fn second_close_and_call_after_close_are_refused() {
    let mut h = Harness::new(2);
    let name = h.open(h.worktree("one")).await;
    let (slow_reply, _slow_answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CallTool {
                session: name.clone(),
                tool: "slow".into(),
                arguments: json!({}),
            },
            slow_reply,
            None,
        )
        .await;
    h.controls.started.notified().await;
    let (close_reply, close_answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: name.clone(),
                summary: "".into(),
            },
            close_reply,
            None,
        )
        .await;
    let limit = std::time::Duration::from_secs(5);
    let second = tokio::time::timeout(
        limit,
        h.request(Request::CloseSession {
            session: name.clone(),
            summary: "".into(),
        }),
    )
    .await
    .expect("a second close must be refused without waiting");
    assert!(refused(second).contains("already closing"));
    let call = tokio::time::timeout(
        limit,
        h.request(Request::CallTool {
            session: name.clone(),
            tool: "click".into(),
            arguments: json!({}),
        }),
    )
    .await
    .expect("a call during a close must be refused without waiting");
    assert!(refused(call).contains("closing"));
    assert!(matches!(
        h.request(Request::Status).await,
        Response::Status(_)
    ));
    h.controls.release.notify_one();
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    h.registry.finished(finished).await;
    assert!(matches!(close_answer.await.unwrap(), Response::Closed(_)));
}

#[tokio::test]
async fn session_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::new(2);
    let name = h.open(h.worktree("one")).await;
    for path in [
        h.registry.dependencies.sessions_dir.clone(),
        h.registry.dependencies.sessions_dir.join(name.to_string()),
    ] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    h.finish(name).await;
}

#[tokio::test]
async fn worktrees_inside_sessions_are_refused_even_through_symlinks() {
    use std::os::unix::fs::symlink;
    let mut h = Harness::new(2);
    let inside = h.registry.dependencies.sessions_dir.join("repo");
    std::fs::create_dir_all(&inside).unwrap();
    let link = h.directory.path().join("link");
    symlink(&inside, &link).unwrap();
    for worktree in [inside, link] {
        let message = refused(
            h.request(Request::OpenSession {
                project: "app".into(),
                worktree,
                goal: "".into(),
                checks: vec![],
            })
            .await,
        );
        assert!(message.contains("sessions"), "{message}");
    }
}
#[tokio::test]
async fn subfolder_of_an_open_worktree_is_refused() {
    let mut h = Harness::new(2);
    let worktree = h.worktree("one");
    let child = worktree.join("child");
    std::fs::create_dir(&child).unwrap();
    let name = h.open(worktree).await;
    let message = refused(
        h.request(Request::OpenSession {
            project: "app".into(),
            worktree: child,
            goal: "".into(),
            checks: vec![],
        })
        .await,
    );
    assert!(message.contains("already open"), "{message}");
    h.finish(name).await;
}

#[tokio::test]
async fn open_refuses_when_host_display_and_signature_are_missing() {
    let mut h = Harness::new(2);
    h.registry.dependencies.host.wayland = None;
    h.registry.dependencies.host.signature = None;
    let worktree = h.worktree("one");
    let message = refused(
        h.request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "".into(),
            checks: vec![],
        })
        .await,
    );
    assert!(
        message.contains("WAYLAND_DISPLAY") && message.contains("HYPRLAND_INSTANCE_SIGNATURE"),
        "{message}"
    );
}

#[tokio::test]
async fn session_names_use_local_time_and_skip_existing_folders() {
    let mut h = Harness::new(2);
    std::fs::create_dir_all(h.registry.dependencies.sessions_dir.join("app_12-30-04")).unwrap();
    let name = h.open(h.worktree("one")).await;
    assert_eq!(name.to_string(), "app_12-30-04-2");
    h.finish(name).await;
}

#[tokio::test(start_paused = true)]
async fn a_stuck_stop_frees_the_slot_after_its_timeout() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("one")).await;
    h.controls
        .stop_hangs
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let (reply, answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: name,
                summary: "".into(),
            },
            reply,
            None,
        )
        .await;
    h.controls.started.notified().await;
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    h.registry.finished(finished).await;
    assert!(matches!(answer.await.unwrap(), Response::Refused { .. }));
    h.controls
        .stop_hangs
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let next = h.open(h.worktree("two")).await;
    h.finish(next).await;
}

#[tokio::test]
async fn a_full_or_closed_mailbox_refuses_without_dropping_the_reply() {
    let (sender, receiver) = mpsc::channel(1);
    let name = "app_12-00-00".parse().unwrap();
    let (first, _answer) = oneshot::channel();
    assert!(send_actor(
        &sender,
        ActorRequest::Close {
            summary: "".into(),
            reply: first
        },
        &name
    ));
    let (reply, answer) = oneshot::channel();
    assert!(!send_actor(
        &sender,
        ActorRequest::Close {
            summary: "".into(),
            reply
        },
        &name
    ));
    assert!(refused(answer.await.unwrap()).contains("busy"));
    drop(receiver);
    let (reply, answer) = oneshot::channel();
    assert!(!send_actor(
        &sender,
        ActorRequest::Close {
            summary: "".into(),
            reply
        },
        &name
    ));
    assert!(refused(answer.await.unwrap()).contains("closed"));
}
