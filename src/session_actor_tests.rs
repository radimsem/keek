use super::*;
use crate::checklist::{IncompleteReason, SessionEnd};
use crate::driver::DriverFuture;
use rmcp::model::{Tool, ToolAnnotations};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Notify;

const ACTION_DURATION_MS: u64 = 200;

struct FakeCompositor;
impl crate::compositor::Compositor for FakeCompositor {
    fn window_events(&self) -> crate::compositor::EventsFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn place_window(
        &self,
        _: Option<crate::compositor::WindowPid>,
        _: crate::compositor::Placement,
        _: Option<crate::compositor::WindowEvents>,
    ) -> crate::compositor::WindowFuture<'_> {
        Box::pin(async { Ok(crate::compositor::WindowAddress("0xabc".into())) })
    }
    fn workspace(&self, _: usize) -> crate::compositor::WorkspaceFuture<'_> {
        Box::pin(async {
            Err(crate::process::ProcessError::Failed {
                program: "fake".into(),
                message: "unused".into(),
            })
        })
    }
    fn install(&self) -> crate::compositor::CompositorFuture<'_> {
        Box::pin(async { Ok(()) })
    }
    fn remove(&self) -> crate::compositor::CompositorFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}
struct FakeClock {
    zero: Instant,
    elapsed: AtomicU64,
}
impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.zero + Duration::from_millis(self.elapsed.load(Ordering::SeqCst))
    }
}
#[path = "session_actor_tests/fake_driver.rs"]
mod fake_driver;
use fake_driver::FakeDriver;
struct FakeFactory(Mutex<Option<FakeDriver>>);
impl DriverFactory for FakeFactory {
    fn open<'a>(
        &'a self,
        _: &'a SessionFolder,
        _: &'a crate::session::Opening,
        _: &'a crate::session::Mounts,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Box<dyn Driver>, SessionError>> + Send + 'a>,
    > {
        Box::pin(
            async move { Ok(Box::new(self.0.lock().unwrap().take().unwrap()) as Box<dyn Driver>) },
        )
    }
}
struct RunningActor {
    window_state: Arc<Mutex<Value>>,
    windows: Arc<Mutex<fake_driver::WindowsReply>>,
    cause: Arc<Mutex<Option<IncompleteReason>>>,
    screenshot: Arc<Mutex<fake_driver::Screenshot>>,
    ended: watch::Sender<()>,
    control: control::Control,
    sender: mpsc::Sender<ActorRequest>,
    close: mpsc::Sender<CloseRequest>,
    task: tokio::task::JoinHandle<Finished>,
    folder: SessionFolder,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    closed: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
    events: watch::Receiver<SessionSnapshot>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    stop_hangs: Arc<AtomicBool>,
    fail_close: Arc<AtomicBool>,
    fail_no_focus: Arc<AtomicBool>,
    host_clipboard: Arc<Mutex<Option<&'static str>>>,
    clock: Arc<FakeClock>,
    render_succeeds: Arc<AtomicBool>,
}
async fn start_actor(root: &Path, id: &str, stop_fails: bool) -> RunningActor {
    let (actor, response) = start_actor_result(root, id, stop_fails, None).await;
    assert!(matches!(response, Response::Opened { .. }));
    actor
}
async fn start_actor_result(
    root: &Path,
    id: &str,
    stop_fails: bool,
    fail_setup: Option<&str>,
) -> (RunningActor, Response) {
    start_actor_with_credentials(root, id, stop_fails, fail_setup, Default::default()).await
}
async fn start_actor_with_credentials(
    root: &Path,
    id: &str,
    stop_fails: bool,
    fail_setup: Option<&str>,
    credentials: crate::credentials::Credentials,
) -> (RunningActor, Response) {
    start_configured(
        root,
        id,
        stop_fails,
        fail_setup,
        credentials,
        Duration::from_secs(300),
        None,
    )
    .await
}
async fn start_configured(
    root: &Path,
    id: &str,
    stop_fails: bool,
    fail_setup: Option<&str>,
    credentials: crate::credentials::Credentials,
    idle_limit: Duration,
    opener: Option<watch::Receiver<()>>,
) -> (RunningActor, Response) {
    start_with(
        root,
        id,
        stop_fails,
        fail_setup,
        credentials,
        idle_limit,
        opener,
        None,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn start_with(
    root: &Path,
    id: &str,
    stop_fails: bool,
    fail_setup: Option<&str>,
    credentials: crate::credentials::Credentials,
    idle_limit: Duration,
    opener: Option<watch::Receiver<()>>,
    delegate: Option<delegate::Delegate>,
) -> (RunningActor, Response) {
    let name: SessionName = id.parse().unwrap();
    let folder = SessionFolder::new(root, &name);
    std::fs::create_dir_all(folder.root()).unwrap();
    let clock = Arc::new(FakeClock {
        zero: Instant::now(),
        elapsed: AtomicU64::new(0),
    });
    let calls = Arc::new(Mutex::new(vec![]));
    let closed = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let stop_hangs = Arc::new(AtomicBool::new(false));
    let fail_close = Arc::new(AtomicBool::new(false));
    let fail_no_focus = Arc::new(AtomicBool::new(false));
    let host_clipboard = Arc::new(Mutex::new(None));
    let (ended, end_receiver) = watch::channel(());
    let screenshot = Arc::new(Mutex::new(fake_driver::Screenshot::Ready));
    let cause = Arc::new(Mutex::new(None));
    let windows = Arc::new(Mutex::new(fake_driver::WindowsReply::Listed));
    let window_state = Arc::new(Mutex::new(
        serde_json::from_str(include_str!("../tests/fixtures/window_state_signin.json")).unwrap(),
    ));
    let driver = FakeDriver {
        window_state: window_state.clone(),
        windows: windows.clone(),
        cause: cause.clone(),
        screenshot: screenshot.clone(),
        ended: end_receiver,
        id: id.into(),
        clock: clock.clone(),
        calls: calls.clone(),
        closed: closed.clone(),
        stop_fails,
        stop_hangs: stop_hangs.clone(),
        fail_setup: fail_setup.map(str::to_owned),
        fail_close: fail_close.clone(),
        entered: entered.clone(),
        release: release.clone(),
    };
    let tools = [
        "read",
        "run_steps",
        "slow",
        "click",
        "get_window_state",
        "type_text",
        "set_value",
    ]
    .into_iter()
    .map(|tool| {
        let schema = json!({"type":"object","properties":{"session":{"type":"string"}}});
        let tool = Tool::new(tool, "fixture", schema.as_object().unwrap().clone());
        if tool.name == "read" {
            tool.with_annotations(ToolAnnotations::new().read_only(true))
        } else {
            tool
        }
    })
    .collect();
    let (events, receiver) = watch::channel(SessionSnapshot {
        control: crate::ipc::ControlState::Verifier,
        state: SessionState::Opening,
        checks: vec![],
        reminded: false,
        last_action: None,
    });
    let render_succeeds = Arc::new(AtomicBool::new(false));
    let (control, control_commands) = control::Control::new();
    let closing = Arc::new(AtomicBool::new(false));
    let actor = Actor {
        test_files: Default::default(),
        frames_dir: root.join("frames"),
        journal: None,
        idle_limit,
        opener,
        control: control.clone(),
        control_commands: Some(control_commands),
        compositor: Arc::new(FakeCompositor),
        window: crate::window_seat::WindowSeat::new(crate::compositor::WindowSpot::Shown {
            x: 24,
            y: 50,
        }),
        opened_at: clock.zero,
        placement: crate::compositor::Placement {
            initial: crate::compositor::WindowSpot::Shown { x: 24, y: 50 },
            monitor: "fake".into(),
            origin: (0, 0),
            geometry: crate::slots::geometry(
                crate::slots::Slot {
                    x: 24,
                    y: 50,
                    width: 1280,
                    height: 800,
                },
                None,
            ),
        },
        credentials,
        project: "app".into(),
        worktree: root.to_owned(),
        profile: Default::default(),
        toolchains: Default::default(),
        warnings: vec![],
        goal: "fixture".into(),
        revision: Revision {
            branch: None,
            commit: "abcdefghi".into(),
            dirty: false,
        },
        started: chrono::Local::now(),
        checklist: crate::checklist::Checklist::new(vec!["fixture works".into()]),
        reminder: Default::default(),
        runner: Arc::new(takeover::Runner {
            inner: EvidenceRunner(root.join("videos"), render_succeeds.clone()),
            calls: calls.clone(),
            fail_no_focus: fail_no_focus.clone(),
            host_clipboard: host_clipboard.clone(),
        }),
        closing: closing.clone(),
        name,
        folder: folder.clone(),
        factory: Arc::new(FakeFactory(Mutex::new(Some(driver)))),
        clock: clock.clone(),
        catalog: Arc::new(Catalog::new(tools)),
        events,
        progress: Default::default(),
        delegate,
    };
    let (sender, mailbox) = mpsc::channel(32);
    let (close, close_receiver) = mpsc::channel(1);
    let (reply, opened) = oneshot::channel();
    let mut task = tokio::spawn(actor.run(Inbox::new(mailbox, close_receiver), reply));
    if fail_setup.is_some() {
        let mut finished = task.await.unwrap();
        finished
            .reply
            .take()
            .unwrap()
            .send(finished.response.clone())
            .unwrap();
        task = tokio::spawn(async move { finished });
    }
    let response = opened.await.unwrap();
    (
        RunningActor {
            window_state,
            windows,
            cause,
            screenshot,
            ended,
            control,
            sender,
            close,
            task,
            folder,
            calls,
            closed,
            closing,
            events: receiver,
            entered,
            release,
            stop_hangs,
            fail_close,
            fail_no_focus,
            host_clipboard,
            clock,
            render_succeeds,
        },
        response,
    )
}
async fn call(actor: &RunningActor, tool: &str) -> Response {
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: tool.into(),
            arguments: json!({"session":"keek-name"}),
            reply,
        })
        .await
        .unwrap();
    response.await.unwrap()
}
async fn close(actor: &RunningActor) -> oneshot::Receiver<Response> {
    let (reply, marked) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::MarkCheck {
            check: crate::timeline::CheckId(1),
            outcome: crate::timeline::CheckOutcome::Passed,
            comment: String::new(),
            reply,
        })
        .await
        .unwrap();
    assert!(matches!(
        marked.await.unwrap(),
        Response::CheckMarked { .. }
    ));
    let (reply, response) = oneshot::channel();
    actor
        .close
        .send(CloseRequest {
            summary: "checked fixture".into(),
            reply,
        })
        .await
        .unwrap();
    response
}
async fn finish(actor: RunningActor) -> Response {
    let response = close(&actor).await;
    let ended = actor.task.await.unwrap();
    ended.reply.unwrap().send(ended.response).unwrap();
    response.await.unwrap()
}

#[path = "session_actor_tests/delegate.rs"]
mod delegate_steps;
#[path = "session_actor_tests/fake_decision_model.rs"]
mod fake_decision_model;
#[path = "session_actor_tests/refusal_hint.rs"]
mod refusal_hint;
#[path = "session_actor_tests/reminder.rs"]
mod reminder;
#[path = "session_actor_tests/timeline.rs"]
mod timeline;

#[tokio::test]
async fn a_slow_call_does_not_block_another_session() {
    let root = tempfile::tempdir().unwrap();
    let slow = start_actor(root.path(), "slow_12-00-00", false).await;
    let fast = start_actor(root.path(), "fast_12-00-00", false).await;
    let (reply, waiting) = oneshot::channel();
    slow.sender
        .send(ActorRequest::Call {
            arrival: slow.control.gate.arrival(),
            tool: "slow".into(),
            arguments: json!({}),
            reply,
        })
        .await
        .unwrap();
    slow.entered.notified().await;
    let response = tokio::time::timeout(Duration::from_secs(1), call(&fast, "click"))
        .await
        .unwrap();
    assert_eq!(
        response,
        Response::ToolResult(json!({"driver":"fast_12-00-00"}))
    );
    slow.release.notify_one();
    assert!(matches!(waiting.await.unwrap(), Response::ToolResult(_)));
    finish(slow).await;
    finish(fast).await;
}

#[tokio::test]
async fn records_replace_symlinks_without_writing_their_targets() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let folder = SessionFolder::new(root.path(), &"app_12-00-00".parse().unwrap());
    std::fs::create_dir(folder.root()).unwrap();
    for name in ["timeline.json.tmp", "summary.txt"] {
        let target = root.path().join(format!("{name}.target"));
        std::fs::write(&target, "unchanged").unwrap();
        symlink(&target, folder.root().join(name)).unwrap();
    }
    write_records(&folder, &SessionTimeline::new(VIDEO_LEAD), "summary")
        .await
        .unwrap();
    for name in ["timeline.json.tmp", "summary.txt"] {
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("{name}.target"))).unwrap(),
            "unchanged"
        );
    }
}

#[tokio::test]
async fn records_replace_fifos_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    let root = tempfile::tempdir().unwrap();
    let folder = SessionFolder::new(root.path(), &"app_12-00-00".parse().unwrap());
    std::fs::create_dir(folder.root()).unwrap();
    for name in ["timeline.json.tmp", "summary.txt"] {
        let path = std::ffi::CString::new(folder.root().join(name).as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    }
    write_records(&folder, &SessionTimeline::new(VIDEO_LEAD), "summary")
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_stuck_driver_call_times_out_and_is_recorded_as_error() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let (reply, answer) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
            arrival: actor.control.gate.arrival(),
            tool: "slow".into(),
            arguments: json!({}),
            reply,
        })
        .await
        .unwrap();
    actor.entered.notified().await;
    tokio::time::advance(Duration::from_secs(151)).await;
    let result = tokio::time::timeout(Duration::from_secs(1), answer).await;
    assert!(
        matches!(result, Ok(Ok(Response::Refused { .. }))),
        "{result:?}"
    );
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused");
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        &timeline.events()[0],
        TimelineEvent::Call { is_error: true, .. }
    ));
}

#[tokio::test]
async fn setup_calls_precede_recording_with_the_expected_arguments() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    let calls = actor.calls.lock().unwrap().clone();
    assert_eq!(
        calls[..3],
        [
            (
                "set_config".into(),
                json!({"cursor.motion.effects.ripple":"on","cursor.badges":"minimal"})
            ),
            (
                "set_agent_cursor_motion".into(),
                json!({"session":"app_12-00-00","effects":{"ripple":"on"},"idle_hide_ms":1000})
            ),
            (
                "start_recording".into(),
                json!({"output_dir":actor.folder.root(),"record_video":true})
            ),
        ]
    );
    call(&actor, "run_steps").await;
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused");
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        &timeline.events()[0],
        TimelineEvent::Call {
            start_ms: Millis(0),
            end_ms: Millis(134),
            ..
        }
    ));
}
#[tokio::test]
async fn an_error_from_any_setup_call_refuses_open_and_closes_the_driver() {
    let root = tempfile::tempdir().unwrap();
    for tool in ["set_config", "set_agent_cursor_motion", "start_recording"] {
        let (actor, response) = start_actor_result(root.path(), tool, false, Some(tool)).await;
        assert!(
            matches!(response, Response::Refused { .. }),
            "{tool}: {response:?}"
        );
        assert!(actor.closed.load(Ordering::SeqCst), "{tool}");
        actor.task.await.unwrap();
    }
}

#[path = "session_actor_tests/editor.rs"]
mod editor;

struct EvidenceRunner(std::path::PathBuf, Arc<AtomicBool>);
impl crate::process::ProcessRunner for EvidenceRunner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            match program {
                "xdg-user-dir" => {
                    assert_eq!(args, ["VIDEOS"]);
                    Ok(self.0.to_string_lossy().into_owned())
                }
                "ffprobe" => Ok("60.0".into()),
                "fc-match" => {
                    assert_eq!(args, ["-f", "%{family}|%{file}", "JetBrains Mono"]);
                    Ok("JetBrains Mono|/usr/share/fonts/fixture.ttf".into())
                }
                "ffmpeg" if args.iter().any(|arg| arg.contains("freezedetect")) => Ok(
                    "lavfi.freezedetect.freeze_start: 0\nlavfi.freezedetect.freeze_end: 60".into(),
                ),
                "ffmpeg" if self.1.load(Ordering::SeqCst) => {
                    std::fs::write(args.last().unwrap(), b"edited recording").unwrap();
                    Ok(String::new())
                }
                "ffmpeg" => Err(crate::process::ProcessError::Failed {
                    program: program.into(),
                    message: "fixture ffmpeg failure".into(),
                }),
                _ => panic!("unexpected program {program}"),
            }
        })
    }
}

#[path = "session_actor_tests/project.rs"]
mod project;

#[path = "session_actor_tests/credentials.rs"]
mod credentials;

#[path = "session_actor_tests/takeover.rs"]
mod takeover;

#[path = "session_actor_tests/recording_keepalive.rs"]
mod recording_keepalive;

#[path = "session_actor_tests/paste.rs"]
mod paste;
#[path = "session_actor_tests/step_away.rs"]
mod step_away;

#[path = "session_actor_tests/queued_help.rs"]
mod queued_help;

#[path = "session_actor_tests/incomplete.rs"]
mod incomplete;

#[path = "session_actor_tests/close.rs"]
mod close_tests;

#[path = "session_actor_tests/host_window.rs"]
mod host_window;

#[path = "session_actor_tests/close_failed.rs"]
mod close_failed;

#[path = "session_actor_tests/withdraw_help.rs"]
mod withdraw_help;

#[path = "session_actor_tests/hand_back.rs"]
mod hand_back;

#[path = "session_actor_tests/hand_back_scale.rs"]
mod hand_back_scale;

#[path = "session_actor_tests/screen_hand_back.rs"]
mod screen_hand_back;
