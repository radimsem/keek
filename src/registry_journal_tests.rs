use super::*;
#[tokio::test]
async fn an_open_journal_tracks_checks_and_calls_and_disappears_after_filing() {
    let mut h = Harness::new(1);
    let path = h.worktree("journal");
    let name = h.open(path).await;
    h.mark(&name).await;
    h.request(Request::AddCheck {
        session: name.clone(),
        text: "added".into(),
    })
    .await;
    h.request(Request::CallTool {
        session: name.clone(),
        tool: "click".into(),
        arguments: json!({}),
    })
    .await;
    let path = h
        .directory
        .path()
        .join("state/sessions")
        .join(format!("{name}.json"));
    let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["checks"][0]["mark"]["outcome"]["outcome"], "passed");
    assert_eq!(value["checks"][1]["added"], true);
    assert_eq!(value["timeline"]["events"].as_array().unwrap().len(), 2);
    h.request(Request::MarkCheck {
        session: name.clone(),
        check: crate::timeline::CheckId(2),
        outcome: crate::timeline::CheckOutcome::Passed,
        comment: String::new(),
    })
    .await;
    assert!(matches!(h.finish_marked(name).await, Response::Closed(_)));
    assert!(!path.exists());
}

#[tokio::test]
async fn recovery_files_marked_checks_without_a_verdict_and_removes_the_journal() {
    let mut h = Harness::new(1);
    let path = h.worktree("recovery");
    let name = h.open(path).await;
    h.mark(&name).await;
    h.request(Request::CallTool {
        session: name.clone(),
        tool: "click".into(),
        arguments: json!({}),
    })
    .await;
    let journal: crate::session_journal::Journal = serde_json::from_slice(
        &std::fs::read(
            h.directory
                .path()
                .join(format!("state/sessions/{name}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    let journaled = journal.timeline.events().to_vec();
    assert!(!journaled.is_empty());
    let tree = h.registry.dependencies.trees.join(name.to_string());
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(
        h.registry
            .dependencies
            .sessions_dir
            .join(name.to_string())
            .join("timeline.json"),
        "untrusted",
    )
    .unwrap();
    h.registry.tasks.abort_all();
    while h.registry.tasks.join_next().await.is_some() {}
    h.registry.recover().await;
    while let Some(result) = h.registry.recovery.join_next().await {
        result.unwrap();
    }
    assert!(!tree.exists());
    let reports = h.directory.path().join("videos/keek/app/2026-01-15");
    let report =
        std::fs::read_to_string(reports.join(format!("{name}_incomplete_report.md"))).unwrap();
    assert!(
        report
            .contains("The keek daemon stopped during the session, and the next daemon filed it.")
    );
    assert!(report.contains("Passed"));
    assert!(!report.contains("Verdict:"));
    assert!(report.contains("- Start: `true`"), "{report}");
    let filed: crate::timeline::SessionTimeline = serde_json::from_slice(
        &std::fs::read(reports.join(format!("{name}_incomplete_timeline.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(filed.events(), journaled);
    assert!(
        !h.directory
            .path()
            .join(format!("state/sessions/{name}.json"))
            .exists()
    );
}

async fn stop_without_filing(h: &mut Harness) {
    h.registry.tasks.abort_all();
    while h.registry.tasks.join_next().await.is_some() {}
}
async fn recover(h: &mut Harness) {
    h.registry.recover().await;
    while let Some(result) = h.registry.recovery.join_next().await {
        result.unwrap();
    }
}
#[tokio::test]
async fn recovery_removes_a_journal_when_its_session_folder_is_gone() {
    let mut h = Harness::new(1);
    let path = h.worktree("missing");
    let name = h.open(path).await;
    stop_without_filing(&mut h).await;
    std::fs::remove_dir_all(h.registry.dependencies.sessions_dir.join(name.to_string())).unwrap();
    recover(&mut h).await;
    assert!(
        !h.directory
            .path()
            .join(format!("state/sessions/{name}.json"))
            .exists()
    );
}

#[tokio::test]
async fn recovery_keeps_the_journal_when_evidence_cannot_be_filed() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("unfiled-recovery")).await;
    stop_without_filing(&mut h).await;
    std::fs::write(h.directory.path().join("videos"), "not a directory").unwrap();
    recover(&mut h).await;
    assert!(
        h.directory
            .path()
            .join(format!("state/sessions/{name}.json"))
            .exists()
    );
}
#[tokio::test]
async fn recovery_rejects_a_project_that_is_not_a_single_name() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("invalid-journal-project")).await;
    stop_without_filing(&mut h).await;
    let path = h
        .directory
        .path()
        .join(format!("state/sessions/{name}.json"));
    let mut journal: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    journal["project"] = json!("../invalid");
    std::fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    recover(&mut h).await;
    assert!(path.exists());
}
#[tokio::test]
async fn recovery_does_not_probe_a_missing_recording() {
    struct ProbeCounter {
        inner: Runner,
        probes: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl ProcessRunner for ProbeCounter {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            cwd: Option<&'a Path>,
        ) -> crate::process::ProcessFuture<'a> {
            if program == "ffprobe" {
                self.probes.fetch_add(1, Ordering::SeqCst);
            }
            self.inner.output(program, args, cwd)
        }
    }
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("missing-recording")).await;
    stop_without_filing(&mut h).await;
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    h.registry.dependencies.runner = Arc::new(ProbeCounter {
        inner: Runner(h.directory.path().join("videos")),
        probes: count.clone(),
    });
    assert!(
        !h.registry
            .dependencies
            .sessions_dir
            .join(name.to_string())
            .join("recording.mp4")
            .exists()
    );
    recover(&mut h).await;
    assert_eq!(count.load(Ordering::SeqCst), 0);
}
async fn credential_recovery(change: Option<Option<&str>>) {
    let mut h = Harness::new(1);
    let env = h.registry.dependencies.projects_config.join("app.env");
    std::fs::create_dir_all(env.parent().unwrap()).unwrap();
    std::fs::write(&env, "PASSWORD=fictional-secret").unwrap();
    let path = h.worktree("credentials");
    let name = h.open(path).await;
    let turn = h
        .registry
        .dependencies
        .sessions_dir
        .join(name.to_string())
        .join("turn-1");
    std::fs::create_dir_all(&turn).unwrap();
    std::fs::write(turn.join("action.json"), r#"{"text":"fictional-secret"}"#).unwrap();
    stop_without_filing(&mut h).await;
    if let Some(change) = change {
        match change {
            Some(text) => std::fs::write(&env, text).unwrap(),
            None => std::fs::remove_file(&env).unwrap(),
        }
    }
    recover(&mut h).await;
    let base = h.directory.path().join("videos/keek/app/2026-01-15");
    let actions = base.join(format!("{name}_incomplete_actions"));
    let report =
        std::fs::read_to_string(base.join(format!("{name}_incomplete_report.md"))).unwrap();
    if change.is_none() {
        let action = std::fs::read_to_string(actions.join("turn-1/action.json")).unwrap();
        assert!(action.contains("[masked]"));
        assert!(!action.contains("fictional-secret"));
    } else {
        assert!(!actions.exists());
        assert!(report.contains("The Project's Test Credentials file changed or was removed after the session, so keek withheld the Action Log."));
        assert!(!report.contains("## Verifier summary"));
        assert!(
            report.contains(
                &h.registry
                    .dependencies
                    .sessions_dir
                    .join(name.to_string())
                    .display()
                    .to_string()
            )
        );
        assert!(report.contains("Check and delete it."));
    }
}
#[tokio::test]
async fn recovery_scrubs_the_action_log_when_test_credentials_are_unchanged() {
    credential_recovery(None).await;
}
#[tokio::test]
async fn recovery_files_the_action_log_when_credentials_were_always_absent() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("no-credentials")).await;
    let folder = h.registry.dependencies.sessions_dir.join(name.to_string());
    std::fs::create_dir(folder.join("turn-1")).unwrap();
    std::fs::write(folder.join("turn-1/action.json"), "fixture").unwrap();
    stop_without_filing(&mut h).await;
    recover(&mut h).await;
    assert!(
        h.directory
            .path()
            .join(format!(
                "videos/keek/app/2026-01-15/{name}_incomplete_actions/turn-1/action.json"
            ))
            .exists()
    );
}
#[tokio::test]
async fn recovery_withholds_the_action_log_when_test_credentials_changed() {
    credential_recovery(Some(Some("PASSWORD=different-secret"))).await;
}
#[tokio::test]
async fn recovery_withholds_the_action_log_when_test_credentials_were_removed() {
    credential_recovery(Some(None)).await;
}
struct RecoveryRunner {
    inner: Runner,
    calls: Arc<Mutex<Vec<Vec<String>>>>,
}
impl ProcessRunner for RecoveryRunner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        if program == "ffprobe" {
            return Box::pin(async move {
                Ok(if args.contains(&"stream=width,height") {
                    "640x480"
                } else {
                    "10.0"
                }
                .into())
            });
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
        _: &'a mut (dyn FnMut(&str) + Send),
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| (*s).to_owned()).collect());
            if !args.contains(&"null") {
                std::fs::write(args.last().unwrap(), b"video").unwrap();
            }
            Ok(String::new())
        })
    }
}
#[tokio::test]
async fn an_open_journal_takeover_excludes_the_rest_of_the_recording() {
    use crate::timeline::Millis;
    let mut h = Harness::new(1);
    let path = h.worktree("takeover");
    let name = h.open(path).await;
    stop_without_filing(&mut h).await;
    let path = h
        .directory
        .path()
        .join(format!("state/sessions/{name}.json"));
    let mut journal: crate::session_journal::Journal =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    journal.takeovers.push(crate::session_journal::Takeover {
        start: Millis(2000),
        end: None,
    });
    std::fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    let recording = h
        .registry
        .dependencies
        .sessions_dir
        .join(name.to_string())
        .join("recording.mp4");
    std::fs::write(recording, b"raw").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    h.registry.dependencies.runner = Arc::new(RecoveryRunner {
        inner: Runner(h.directory.path().join("videos")),
        calls: calls.clone(),
    });
    recover(&mut h).await;
    let base = h.directory.path().join("videos/keek/app/2026-01-15");
    assert!(
        base.join(format!("{name}_incomplete_verification.mp4"))
            .is_file()
    );
    let filed: crate::timeline::SessionTimeline = serde_json::from_slice(
        &std::fs::read(base.join(format!("{name}_incomplete_timeline.json"))).unwrap(),
    )
    .unwrap();
    assert!(filed.events().iter().any(|event| matches!(
        event,
        crate::timeline::TimelineEvent::Takeover {
            start_ms: Millis(2000),
            end_ms: Millis(10000),
            ..
        }
    )));
    let calls = calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|args| args.iter().any(|arg| arg.contains("You were in control")))
    );
    let raw_segments: Vec<_> = calls
        .iter()
        .filter(|args| {
            args.iter().any(|arg| arg.ends_with("recording.mp4"))
                && args.contains(&"-ss".to_owned())
                && !args.iter().any(|arg| arg.contains("gblur="))
        })
        .collect();
    assert!(!raw_segments.is_empty());
    assert!(raw_segments.iter().all(|args| {
        let start = args.iter().position(|arg| arg == "-ss").unwrap();
        let duration = args.iter().position(|arg| arg == "-t").unwrap();
        args[start + 1].parse::<f64>().unwrap() + args[duration + 1].parse::<f64>().unwrap() <= 2.0
    }));
}

#[tokio::test]
async fn recovery_uses_recorded_takeovers_instead_of_rounded_journal_spans() {
    use crate::timeline::{Millis, TimelineEvent};
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("rounded-takeover")).await;
    stop_without_filing(&mut h).await;
    let path = h
        .directory
        .path()
        .join(format!("state/sessions/{name}.json"));
    let mut journal: crate::session_journal::Journal =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    journal.takeovers.push(crate::session_journal::Takeover {
        start: Millis(999),
        end: None,
    });
    journal.timeline.push(TimelineEvent::Takeover {
        reason: None,
        start_ms: Millis(1000),
        end_ms: Millis(2000),
    });
    let timeline = journal.recovery_timeline(Millis(10000));
    assert_eq!(
        timeline
            .events()
            .iter()
            .filter(|event| matches!(event, TimelineEvent::Takeover { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn the_daemon_waits_for_recovery_then_exits_when_idle() {
    struct BlockedRunner {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        inner: Runner,
    }
    impl ProcessRunner for BlockedRunner {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            cwd: Option<&'a Path>,
        ) -> crate::process::ProcessFuture<'a> {
            Box::pin(async move {
                if args == ["VIDEOS"] {
                    self.entered.notify_one();
                    self.release.notified().await;
                }
                self.inner.output(program, args, cwd).await
            })
        }
    }
    let mut h = Harness::new(1);
    let path = h.worktree("alive");
    h.open(path).await;
    stop_without_filing(&mut h).await;
    h.registry.sessions.clear();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    h.registry.dependencies.runner = Arc::new(BlockedRunner {
        entered: entered.clone(),
        release: release.clone(),
        inner: Runner(h.directory.path().join("videos")),
    });
    h.registry.recover().await;
    let listener = tokio::net::UnixListener::bind(h.directory.path().join("daemon.sock")).unwrap();
    let task = tokio::spawn(crate::daemon::serve(
        listener,
        crate::daemon::IdleWatch::new(
            crate::clock::MonotonicClock,
            std::time::Duration::from_millis(20),
        ),
        h.registry,
        std::future::pending(),
    ));
    entered.notified().await;
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    assert!(!task.is_finished());
    release.notify_one();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        crate::daemon::Exit::Idle
    );
}

struct ControlRunner(Runner);
impl ProcessRunner for ControlRunner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            if program == "hyprctl" {
                return Ok(match args.first().copied() {
                    Some("clients") => {
                        r#"[{"class":"editor","address":"0xdef","focusHistoryID":0}]"#
                    }
                    Some("binds") => "[]",
                    Some("activewindow") => r#"{"address":"0xdef"}"#,
                    _ => "ok",
                }
                .into());
            }
            self.0.output(program, args, cwd).await
        })
    }
}
#[tokio::test]
async fn takeover_changes_are_journaled_while_a_help_request_waits() {
    let mut h = Harness::new(1);
    h.registry.dependencies.runner =
        Arc::new(ControlRunner(Runner(h.directory.path().join("videos"))));
    let worktree = h.worktree("help-journal");
    let name = h.open(worktree).await;
    let (reply, mut answer) = oneshot::channel();
    h.registry
        .handle(
            Request::RequestHelp {
                session: name.clone(),
                reason: "Sign in".into(),
            },
            reply,
            None,
        )
        .await;
    let mut events = h.registry.sessions.get(&name).unwrap().events.clone();
    let path = h
        .directory
        .path()
        .join(format!("state/sessions/{name}.json"));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !matches!(
            events.borrow_and_update().control,
            crate::ipc::ControlState::HelpRequest { .. }
        ) {
            events.changed().await.unwrap();
        }
        h.registry
            .control(crate::viewer::ViewerCommand::TakeControl(name.clone()))
            .await;
        loop {
            let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            if value["takeovers"]
                .as_array()
                .is_some_and(|spans| !spans.is_empty())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(answer.try_recv().is_err());
        h.registry
            .control(crate::viewer::ViewerCommand::Done(name.clone()))
            .await;
        loop {
            let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            if !value["takeovers"][0]["end"].is_null() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    stop_without_filing(&mut h).await;
}

#[path = "registry_journal_planted_tests.rs"]
mod planted;

#[tokio::test]
async fn a_failed_recording_stop_removes_the_journal_after_filing() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("failed-stop-journal")).await;
    let path = h
        .directory
        .path()
        .join(format!("state/sessions/{name}.json"));
    assert!(path.exists());
    h.controls
        .fail_stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let response = h.finish(name).await;
    assert!(
        matches!(response, Response::Closed(value) if value["incomplete"] == crate::checklist::IncompleteReason::RecordingNotStopped.sentence() && value.get("verdict").is_none())
    );
    assert!(!path.exists());
    assert!(h.registry.is_empty());
}
