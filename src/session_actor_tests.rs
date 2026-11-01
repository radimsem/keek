use super::*;
use crate::driver::DriverFuture;
use rmcp::model::{Tool, ToolAnnotations};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Notify;

struct FakeClock {
    zero: Instant,
    elapsed: AtomicU64,
}
impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.zero + Duration::from_millis(self.elapsed.load(Ordering::SeqCst))
    }
}
struct FakeDriver {
    id: String,
    clock: Arc<FakeClock>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    closed: Arc<AtomicBool>,
    stop_fails: bool,
    stop_hangs: Arc<AtomicBool>,
    fail_setup: Option<String>,
    fail_close: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
impl Driver for FakeDriver {
    fn list_tools(&mut self) -> DriverFuture<'_, Vec<Tool>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn call_tool<'a>(&'a mut self, tool: &'a str, arguments: Value) -> DriverFuture<'a, Value> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((tool.to_owned(), arguments));
            if tool == "slow"
                || (tool == "stop_recording" && self.stop_hangs.load(Ordering::SeqCst))
            {
                self.entered.notify_one();
                self.release.notified().await;
            }
            let duration = match tool {
                "set_config" | "set_agent_cursor_motion" => 200,
                "start_recording" | "read" => 50,
                "run_steps" => 200,
                _ => 0,
            };
            self.clock.elapsed.fetch_add(duration, Ordering::SeqCst);
            if self.fail_setup.as_deref() == Some(tool) {
                return Ok(json!({"isError":true}));
            }
            if tool == "stop_recording" && self.stop_fails {
                return Ok(
                    json!({"isError":true,"content":[{"type":"text","text":"fixture recording failure"}]}),
                );
            }
            Ok(json!({"driver":self.id}))
        })
    }
    fn close(&mut self) -> DriverFuture<'_, ()> {
        Box::pin(async {
            self.closed.store(true, Ordering::SeqCst);
            if self.fail_close.load(Ordering::SeqCst) {
                return Err(DriverError::Close("fixture close failure".into()));
            }
            Ok(())
        })
    }
}
struct FakeFactory(Mutex<Option<FakeDriver>>);
impl DriverFactory for FakeFactory {
    fn open<'a>(
        &'a self,
        _: &'a SessionFolder,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Box<dyn Driver>, SessionError>> + Send + 'a>,
    > {
        Box::pin(
            async move { Ok(Box::new(self.0.lock().unwrap().take().unwrap()) as Box<dyn Driver>) },
        )
    }
}
struct RunningActor {
    sender: mpsc::Sender<ActorRequest>,
    task: tokio::task::JoinHandle<Finished>,
    folder: SessionFolder,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    closed: Arc<AtomicBool>,
    events: watch::Receiver<SessionSnapshot>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    stop_hangs: Arc<AtomicBool>,
    fail_close: Arc<AtomicBool>,
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
    let driver = FakeDriver {
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
    let tools = ["read", "run_steps", "slow", "click"]
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
        state: SessionState::Opening,
        checks: vec![],
        last_action: None,
    });
    let actor = Actor {
        project: "app".into(),
        goal: "fixture".into(),
        revision: Revision {
            branch: None,
            commit: "abcdefghi".into(),
            dirty: false,
        },
        started: chrono::Local::now(),
        checklist: crate::checklist::Checklist::new(vec!["fixture works".into()]),
        runner: Arc::new(EvidenceRunner(root.join("videos"))),
        closing: Arc::new(AtomicBool::new(false)),
        name,
        folder: folder.clone(),
        factory: Arc::new(FakeFactory(Mutex::new(Some(driver)))),
        clock,
        catalog: Arc::new(Catalog::new(tools)),
        events,
    };
    let (sender, mailbox) = mpsc::channel(8);
    let (reply, opened) = oneshot::channel();
    let mut task = tokio::spawn(actor.run(mailbox, reply));
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
            sender,
            task,
            folder,
            calls,
            closed,
            events: receiver,
            entered,
            release,
            stop_hangs,
            fail_close,
        },
        response,
    )
}
async fn call(actor: &RunningActor, tool: &str) -> Response {
    let (reply, response) = oneshot::channel();
    actor
        .sender
        .send(ActorRequest::Call {
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
        .sender
        .send(ActorRequest::Close {
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

#[tokio::test]
async fn the_timeline_records_each_call_once_and_excludes_setup() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    call(&actor, "read").await;
    call(&actor, "run_steps").await;
    assert_eq!(
        actor.events.borrow().last_action,
        Some(LastAction {
            tool: "run_steps".into(),
            time_ms: Millis(0)
        })
    );
    let folder = actor.folder.clone();
    let calls = actor.calls.clone();
    let Response::Closed(value) = finish(actor).await else {
        panic!("close refused");
    };
    let timeline = SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        timeline.events(),
        [
            TimelineEvent::Call {
                tool: "read".into(),
                action: false,
                start_ms: Millis(0),
                end_ms: Millis(0),
                is_error: false
            },
            TimelineEvent::Call {
                tool: "run_steps".into(),
                action: true,
                start_ms: Millis(0),
                end_ms: Millis(184),
                is_error: false
            },
            TimelineEvent::CheckMark {
                check: crate::timeline::CheckId(1),
                outcome: crate::timeline::CheckOutcome::Passed,
                at_ms: Millis(184),
            },
        ]
    );
    let calls = calls.lock().unwrap();
    let recording = calls
        .iter()
        .find(|(tool, _)| tool == "start_recording")
        .unwrap();
    assert_eq!(
        recording.1,
        json!({"output_dir":folder.root(),"record_video":true})
    );
    assert!(
        calls
            .iter()
            .filter(|(tool, _)| tool == "read" || tool == "run_steps")
            .all(|(_, args)| args.get("session").is_none())
    );
}

#[tokio::test]
async fn close_writes_the_timeline_and_closes_the_driver_when_stop_recording_fails() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", true).await;
    call(&actor, "click").await;
    let folder = actor.folder.clone();
    let closed = actor.closed.clone();
    let calls = actor.calls.clone();
    assert!(
        matches!(finish(actor).await, Response::Refused { message } if message.contains("fixture recording failure"))
    );
    assert!(closed.load(Ordering::SeqCst));
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "stop_recording")
    );
    let timeline =
        SessionTimeline::parse(&std::fs::read_to_string(folder.timeline()).unwrap()).unwrap();
    assert_eq!(timeline.events().len(), 2);
    assert!(!folder.root().join("timeline.json.tmp").exists());
}

#[tokio::test]
async fn a_slow_call_does_not_block_another_session() {
    let root = tempfile::tempdir().unwrap();
    let slow = start_actor(root.path(), "slow_12-00-00", false).await;
    let fast = start_actor(root.path(), "fast_12-00-00", false).await;
    let (reply, waiting) = oneshot::channel();
    slow.sender
        .send(ActorRequest::Call {
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

#[tokio::test(start_paused = true)]
async fn a_stuck_stop_times_out_and_closes_the_driver() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", false).await;
    actor.stop_hangs.store(true, Ordering::SeqCst);
    let response = close(&actor).await;
    actor.entered.notified().await;
    tokio::time::advance(Duration::from_secs(31)).await;
    let finished = tokio::time::timeout(Duration::from_secs(1), actor.task).await;
    let finished = finished.expect("actor did not finish").unwrap();
    assert!(actor.closed.load(Ordering::SeqCst));
    finished.reply.unwrap().send(finished.response).unwrap();
    assert!(matches!(response.await.unwrap(), Response::Refused { .. }));
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

#[tokio::test]
async fn close_reports_both_stop_and_close_failures() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "app_12-00-00", true).await;
    actor.fail_close.store(true, Ordering::SeqCst);
    let response = finish(actor).await;
    let Response::Refused { message } = response else {
        panic!("{response:?}")
    };
    assert!(
        message.contains("fixture recording failure") && message.contains("fixture close failure"),
        "{message}"
    );
}

struct EvidenceRunner(std::path::PathBuf);
impl crate::process::ProcessRunner for EvidenceRunner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            assert_eq!(program, "xdg-user-dir");
            assert_eq!(args, ["VIDEOS"]);
            Ok(self.0.to_string_lossy().into_owned())
        })
    }
}
