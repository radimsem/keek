use super::*;

async fn serve_until_terminated(h: Harness, expected: Vec<&'static str>) {
    let listener = tokio::net::UnixListener::bind(h.directory.path().join("daemon.sock")).unwrap();
    let compositor = h.compositor.clone();
    let before_termination = expected.clone();
    let terminated = async {
        assert_eq!(*compositor.0.lock().unwrap(), before_termination);
    };
    let exit = crate::daemon::serve(
        listener,
        crate::daemon::IdleWatch::new(
            crate::clock::MonotonicClock,
            std::time::Duration::from_secs(60),
        ),
        h.registry,
        terminated,
    )
    .await
    .unwrap();
    assert_eq!(exit, crate::daemon::Exit::Terminated);
    let mut expected = expected;
    expected.push("remove");
    assert_eq!(*compositor.0.lock().unwrap(), expected);
}

#[tokio::test]
async fn serve_removes_the_host_rule_before_polling_for_requests_and_on_termination() {
    serve_until_terminated(Harness::new(1), vec!["remove"]).await;
}

#[tokio::test]
async fn termination_removes_the_host_rule_even_with_a_session_open() {
    let mut h = Harness::new(1);
    h.open(h.worktree("one")).await;
    assert!(h.registry.keeps_daemon_alive());
    serve_until_terminated(h, vec!["install", "remove"]).await;
}

#[tokio::test]
async fn a_failed_host_rule_removal_does_not_prevent_serving_or_termination() {
    let h = Harness::new(1);
    h.compositor.1.store(true, Ordering::SeqCst);
    serve_until_terminated(h, vec!["remove"]).await;
}

#[tokio::test]
async fn removing_the_host_rule_calls_the_compositor_and_tolerates_failure() {
    let h = Harness::new(1);
    h.registry.remove_host_rule().await;
    h.compositor.1.store(true, Ordering::SeqCst);
    h.registry.remove_host_rule().await;
    assert_eq!(*h.compositor.0.lock().unwrap(), ["remove", "remove"]);
}
