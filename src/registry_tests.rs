use super::*;
use crate::compositor::CompositorFuture;
use crate::driver::{Driver, DriverFuture};
use crate::process::ProcessError;
use chrono::TimeZone;
use rmcp::model::Tool;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Mutex;

type Calls = Arc<Mutex<Vec<(PathBuf, String, Value)>>>;
struct FakeDriver {
    folder: PathBuf,
    calls: Calls,
    controls: Arc<Controls>,
}
impl Driver for FakeDriver {
    fn list_tools(&mut self) -> DriverFuture<'_, Vec<Tool>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn call_tool<'a>(&'a mut self, tool: &'a str, args: Value) -> DriverFuture<'a, Value> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((self.folder.clone(), tool.into(), args));
            if tool == "slow"
                || (tool == "stop_recording"
                    && self
                        .controls
                        .stop_hangs
                        .load(std::sync::atomic::Ordering::Relaxed))
            {
                self.controls.started.notify_one();
                self.controls.release.notified().await;
            }
            if tool == "stop_recording"
                && self
                    .controls
                    .fail_stop
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err(crate::driver::DriverError::Call {
                    tool: tool.into(),
                    reason: "fixture stop failure".into(),
                });
            }
            Ok(json!({"content":[]}))
        })
    }
    fn close(&mut self) -> DriverFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}
#[derive(Default)]
struct Controls {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    fail_stop: std::sync::atomic::AtomicBool,
    stop_hangs: std::sync::atomic::AtomicBool,
    fail_open: std::sync::atomic::AtomicBool,
}
struct Factory(Calls, Arc<Controls>);
impl DriverFactory for Factory {
    fn open<'a>(
        &'a self,
        folder: &'a SessionFolder,
        _: &'a crate::session::Opening,
        _: &'a crate::session::Mounts,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Box<dyn Driver>, SessionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            if self
                .1
                .fail_open
                .swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                return Err(SessionError::Refused("fixture open failed".into()));
            }
            Ok(Box::new(FakeDriver {
                folder: folder.root().to_owned(),
                calls: self.0.clone(),
                controls: self.1.clone(),
            }) as Box<dyn Driver>)
        })
    }
}
struct Runner(PathBuf);
impl ProcessRunner for Runner {
    fn output<'a>(
        &'a self,
        _: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, ProcessError>> + Send + 'a>,
    > {
        Box::pin(async move {
            if args == ["VIDEOS"] {
                return Ok(self.0.to_string_lossy().into_owned());
            }
            if args.ends_with(&["rev-parse", "--show-toplevel"]) {
                let cwd = cwd.unwrap();
                let root = if cwd.file_name().is_some_and(|name| name == "child") {
                    cwd.parent().unwrap()
                } else {
                    cwd
                };
                return Ok(root.to_string_lossy().into_owned());
            }
            Ok(if args.contains(&"symbolic-ref") {
                "feature"
            } else if args.contains(&"status") {
                " M file"
            } else {
                "fake-commit"
            }
            .into())
        })
    }
}
#[derive(Default)]
struct Compositor(Mutex<Vec<&'static str>>, AtomicBool);
impl crate::compositor::Compositor for Compositor {
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
    fn slot(
        &self,
        index: crate::viewer::SlotId,
        _: usize,
        _: Option<crate::project::WindowSize>,
    ) -> crate::compositor::SlotFuture<'_> {
        Box::pin(async move {
            Ok(crate::compositor::Placement {
                slot: crate::slots::Slot {
                    x: index.0 as i32 * 1701 + 24,
                    y: 50,
                    width: 1691,
                    height: 1298,
                },
                monitor: "fake".into(),
                origin: (0, 0),
            })
        })
    }
    fn install(&self) -> CompositorFuture<'_> {
        Box::pin(async {
            self.0.lock().unwrap().push("install");
            Ok(())
        })
    }
    fn remove(&self) -> CompositorFuture<'_> {
        Box::pin(async {
            self.0.lock().unwrap().push("remove");
            if self.1.load(Ordering::SeqCst) {
                Err(ProcessError::Failed {
                    program: "fake compositor".into(),
                    message: "rule removal failed".into(),
                })
            } else {
                Ok(())
            }
        })
    }
}
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
    fn local_time(&self) -> chrono::DateTime<chrono::Local> {
        chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 12, 30, 4)
            .single()
            .unwrap()
    }
}
struct Harness {
    registry: Registry,
    directory: tempfile::TempDir,
    calls: Calls,
    compositor: Arc<Compositor>,
    controls: Arc<Controls>,
}
impl Harness {
    fn new(cap: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("config")).unwrap();
        std::fs::write(
            directory.path().join("config/app.toml"),
            "start = \"true\"\n",
        )
        .unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let compositor = Arc::new(Compositor::default());
        let controls = Arc::new(Controls::default());
        let tools = ["click", "slow"]
            .into_iter()
            .map(|name| Tool::new(name, "fixture", serde_json::Map::new()))
            .collect();
        let (_, catalog) = watch::channel(Some(Ok(Arc::new(Catalog::new(tools)))));
        let registry = Registry::new(
            Dependencies {
                frames_dir: directory.path().join("frames"),
                journals_dir: directory.path().join("state/sessions"),
                session_idle_limit: std::time::Duration::from_secs(300),
                sessions_dir: directory.path().join("sessions"),
                projects_config: directory.path().join("config"),
                projects_cache: directory.path().join("projects"),
                trees: directory.path().join("trees"),
                cap,
                viewer: Arc::new(crate::viewer::NoopViewer),
                host: HostEnvironment {
                    wayland: Some("fake".into()),
                    signature: Some("fake".into()),
                },
                clock: Arc::new(FixedClock),
                runner: Arc::new(Runner(directory.path().join("videos"))),
                compositor: compositor.clone(),
                factory: Arc::new(Factory(calls.clone(), controls.clone())),
            },
            catalog,
        );
        Self {
            registry,
            directory,
            calls,
            compositor,
            controls,
        }
    }
    fn worktree(&self, name: &str) -> PathBuf {
        let path = self.directory.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        // The fake runner names a folder called child's parent as the git top level.
        let top = if path.file_name().is_some_and(|name| name == "child") {
            path.parent().unwrap()
        } else {
            &path
        };
        std::fs::create_dir_all(top.join(".git")).unwrap();
        path
    }
    async fn request(&mut self, request: Request) -> Response {
        let (reply, mut answer) = oneshot::channel();
        self.registry.handle(request, reply, None).await;
        loop {
            tokio::select! {
                response = &mut answer => return response.unwrap(),
                Some(prepared) = self.registry.preparing.join_next() => self.registry.prepared(prepared.unwrap()).await,
                Some(finished) = self.registry.tasks.join_next() => self.registry.finished(finished.unwrap()).await,
            }
        }
    }
    async fn open(&mut self, worktree: PathBuf) -> SessionName {
        let response = self
            .request(Request::OpenSession {
                project: "app".into(),
                worktree,
                goal: "goal".into(),
                checks: vec!["check".into()],
            })
            .await;
        let Response::Opened { session, .. } = response else {
            panic!("open failed: {response:?}");
        };
        session
    }
    async fn mark(&mut self, name: &SessionName) {
        assert!(matches!(
            self.request(Request::MarkCheck {
                session: name.clone(),
                check: crate::timeline::CheckId(1),
                outcome: crate::timeline::CheckOutcome::Passed,
                comment: String::new()
            })
            .await,
            Response::CheckMarked { .. }
        ));
    }
    async fn finish(&mut self, name: SessionName) -> Response {
        self.mark(&name).await;
        self.finish_marked(name).await
    }
    async fn finish_marked(&mut self, name: SessionName) -> Response {
        let (reply, answer) = oneshot::channel();
        self.registry
            .handle(
                Request::CloseSession {
                    session: name,
                    summary: "verified".into(),
                },
                reply,
                None,
            )
            .await;
        let finished = self.registry.tasks.join_next().await.unwrap().unwrap();
        self.registry.finished(finished).await;
        answer.await.unwrap()
    }
}
fn refused(response: Response) -> String {
    let Response::Refused { message } = response else {
        panic!("not a refusal: {response:?}");
    };
    message
}

#[tokio::test]
async fn calls_reach_only_the_named_sessions_driver() {
    let mut h = Harness::new(2);
    let first = h.open(h.worktree("one")).await;
    let second = h.open(h.worktree("two")).await;
    assert_eq!(first.to_string(), "app_12-30-04");
    assert_eq!(second.to_string(), "app_12-30-04-2");
    for (session, x) in [(&first, 10), (&second, 20)] {
        assert!(matches!(
            h.request(Request::CallTool {
                session: session.clone(),
                tool: "click".into(),
                arguments: json!({"x":x})
            })
            .await,
            Response::ToolResult(_)
        ));
    }
    {
        let calls = h.calls.lock().unwrap();
        let clicks: Vec<_> = calls
            .iter()
            .filter(|(_, tool, _)| tool == "click")
            .collect();
        assert_eq!(clicks.len(), 2);
        assert!(clicks[0].0.ends_with(first.to_string()));
        assert_eq!(clicks[0].2, json!({"x":10}));
        assert!(clicks[1].0.ends_with(second.to_string()));
        assert_eq!(clicks[1].2, json!({"x":20}));
    }
    assert_eq!(*h.compositor.0.lock().unwrap(), ["install"]);
    h.finish(first).await;
    h.finish(second).await;
    assert_eq!(*h.compositor.0.lock().unwrap(), ["install", "remove"]);
}

#[tokio::test]
async fn a_symlink_cannot_open_a_second_session_for_the_same_worktree() {
    let mut h = Harness::new(2);
    let path = h.worktree("worktree");
    let name = h.open(path.clone()).await;
    let alias = h.directory.path().join("alias");
    std::os::unix::fs::symlink(path, &alias).unwrap();
    let message = refused(
        h.request(Request::OpenSession {
            project: "app".into(),
            worktree: alias,
            goal: "goal".into(),
            checks: vec![],
        })
        .await,
    );
    assert!(message.contains("already open"), "{message}");
    assert_eq!(h.registry.len(), 1);
    h.finish(name).await;
}

#[tokio::test]
async fn the_cap_refuses_another_worktree() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("one")).await;
    let worktree = h.worktree("two");
    let message = refused(
        h.request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec![],
        })
        .await,
    );
    assert!(message.contains("cap 1"), "{message}");
    h.finish(name).await;
}

#[tokio::test]
async fn an_unknown_session_names_the_open_sessions() {
    let mut h = Harness::new(2);
    let name = h.open(h.worktree("one")).await;
    let message = refused(
        h.request(Request::CallTool {
            session: "missing".parse().unwrap(),
            tool: "click".into(),
            arguments: json!({}),
        })
        .await,
    );
    assert!(message.contains(&name.to_string()), "{message}");
    h.finish(name).await;
}

#[tokio::test]
async fn accepted_shutdown_refuses_every_later_open() {
    let mut h = Harness::new(2);
    assert_eq!(h.registry.shutdown(), Response::ShuttingDown);
    let worktree = h.worktree("one");
    let message = refused(
        h.request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec![],
        })
        .await,
    );
    assert!(message.contains("ShuttingDown"), "{message}");
    assert!(h.registry.is_empty());
}

#[tokio::test]
async fn close_files_the_timeline_and_revision_in_the_report() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("one")).await;
    h.request(Request::CallTool {
        session: name.clone(),
        tool: "click".into(),
        arguments: json!({}),
    })
    .await;
    let folder = SessionFolder::new(&h.registry.dependencies.sessions_dir, &name);
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.root().join("session.json")).unwrap())
            .unwrap();
    assert_eq!(record["goal"], "goal");
    assert_eq!(record["checks"], json!(["check"]));
    assert_eq!(record["revision"]["dirty"], true);
    let response = h.finish(name.clone()).await;
    let Response::Closed(value) = response else {
        panic!("close refused");
    };
    let timeline = crate::timeline::SessionTimeline::parse(
        &std::fs::read_to_string(value["evidence"]["timeline"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(timeline.events().len(), 2);
    assert!(!folder.root().exists());
    let report = std::fs::read_to_string(value["evidence"]["report"].as_str().unwrap()).unwrap();
    assert!(report.contains("# goal"));
    assert!(report.contains("Verified feature at fake-co + uncommitted changes"));
    assert!(report.contains("1. check: Passed"));
    assert!(report.contains("- Start: `true`"), "{report}");

    assert!(
        h.calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, tool, _)| tool == "stop_recording")
    );
}

#[tokio::test]
async fn a_slow_call_in_one_session_does_not_block_another() {
    let mut h = Harness::new(2);
    let first = h.open(h.worktree("one")).await;
    let second = h.open(h.worktree("two")).await;
    let (reply, mut answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CallTool {
                session: first.clone(),
                tool: "slow".into(),
                arguments: json!({}),
            },
            reply,
            None,
        )
        .await;
    h.controls.started.notified().await;
    let quick = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        h.request(Request::CallTool {
            session: second.clone(),
            tool: "click".into(),
            arguments: json!({}),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(quick, Response::ToolResult(_)));
    assert!(answer.try_recv().is_err());
    h.controls.release.notify_one();
    assert!(matches!(answer.await.unwrap(), Response::ToolResult(_)));
    h.finish(first).await;
    h.finish(second).await;
}

#[tokio::test]
async fn a_recording_stop_failure_still_releases_the_slot_and_writes_timeline() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("one")).await;
    h.controls
        .fail_stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let response = h.finish(name.clone()).await;
    let Response::Closed(value) = response else {
        panic!("expected Closed")
    };
    assert_eq!(
        value["incomplete"],
        crate::checklist::IncompleteReason::RecordingNotStopped.sentence()
    );
    assert!(value.get("verdict").is_none());
    assert!(h.registry.is_empty());
    assert!(Path::new(value["evidence"]["timeline"].as_str().unwrap()).exists());
    assert_eq!(*h.compositor.0.lock().unwrap(), ["install", "remove"]);
}

#[tokio::test]
async fn opening_sessions_count_while_the_catalog_is_pending() {
    let mut h = Harness::new(1);
    let (catalog_sender, catalog) = watch::channel(None);
    h.registry.catalog = catalog;
    let (reply, answer) = oneshot::channel();
    let worktree = h.worktree("one");
    h.registry
        .handle(
            Request::OpenSession {
                project: "app".into(),
                worktree,
                goal: "goal".into(),
                checks: vec!["check".into()],
            },
            reply,
            None,
        )
        .await;
    assert_eq!(h.registry.len(), 1);
    assert_eq!(
        h.registry.shutdown(),
        Response::ShutdownRefused { open_sessions: 1 }
    );
    let Response::Status(status) = h.request(Request::Status).await else {
        panic!("no status");
    };
    assert_eq!(status.open_sessions, 1);
    catalog_sender.send_replace(Some(Ok(Arc::new(Catalog::default()))));
    let prepared = h.registry.preparing.join_next().await.unwrap().unwrap();
    h.registry.prepared(prepared).await;
    let Response::Opened { session, .. } = answer.await.unwrap() else {
        panic!("open failed");
    };
    h.finish(session).await;
}

#[tokio::test]
async fn a_closing_session_keeps_refusing_calls_before_the_actor_replies() {
    let mut h = Harness::new(2);
    let session = h.open(h.worktree("one")).await;
    h.mark(&session).await;
    let (reply, answer) = oneshot::channel();
    h.registry
        .handle(
            Request::CloseSession {
                session: session.clone(),
                summary: "done".into(),
            },
            reply,
            None,
        )
        .await;
    let response = h
        .request(Request::CallTool {
            session: session.clone(),
            tool: "click".into(),
            arguments: json!({}),
        })
        .await;
    assert!(matches!(response, Response::Refused { .. }), "{response:?}");
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    h.registry.finished(finished).await;
    answer.await.unwrap();
}

#[path = "registry_lifecycle_tests.rs"]
mod lifecycle;

#[path = "registry_check_tests.rs"]
mod checks;

#[path = "registry_project_tests.rs"]
mod project;

#[path = "registry_progress_tests.rs"]
mod progress;

#[path = "registry_viewer_tests.rs"]
mod viewer;

#[path = "registry_incomplete_tests.rs"]
mod incomplete;

#[path = "registry_journal_tests.rs"]
mod journal;

#[path = "registry_daemon_tests.rs"]
mod daemon;

#[path = "registry_profile_tests.rs"]
mod profile;
