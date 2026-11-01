use crate::catalog::Catalog;
use crate::checklist::Checklist;
use crate::clock::Clock;
use crate::driver::{Driver, DriverError};
use crate::ipc::{CheckStatus, LastAction, Response, SessionSnapshot, SessionState};
use crate::session_folder::{SessionFolder, SessionName};
use crate::timeline::{CheckId, CheckOutcome};
use crate::timeline::{Millis, SessionTimeline, TimelineEvent};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot, watch};

const VIDEO_LEAD: Millis = Millis(116);
fn timeline_time(zero: Instant, time: Instant) -> Millis {
    Millis((time.saturating_duration_since(zero).as_millis() as u64).saturating_sub(VIDEO_LEAD.0))
}
#[derive(Clone)]
pub struct HostEnvironment {
    pub wayland: Option<String>,
    pub signature: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Sandbox(#[from] crate::sandbox::SandboxError),
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Driver(#[from] DriverError),
    #[error("could not start pasta: {0}. Check that pasta is installed and retry.")]
    SpawnPasta(std::io::Error),
    #[error("could not write Agent Session records: {0}. Check the session folder permissions.")]
    Io(#[from] std::io::Error),
    #[error("could not serialize Agent Session records: {0}. Check the session folder.")]
    Json(#[from] serde_json::Error),
}
impl HostEnvironment {
    pub fn capture() -> Self {
        Self {
            wayland: std::env::var("WAYLAND_DISPLAY").ok(),
            signature: std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok(),
        }
    }
    pub fn validate(&self) -> Result<(), SessionError> {
        let missing: Vec<_> = [
            ("WAYLAND_DISPLAY", &self.wayland),
            ("HYPRLAND_INSTANCE_SIGNATURE", &self.signature),
        ]
        .into_iter()
        .filter(|(_, v)| v.as_ref().is_none_or(String::is_empty))
        .map(|(k, _)| k)
        .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(SessionError::Refused(format!(
                "Cannot open an Agent Session because {} is missing. Start keek from the host Hyprland desktop.",
                missing.join(", ")
            )))
        }
    }
}
pub fn choose_name(
    project: &str,
    time: &str,
    exists: impl Fn(&str) -> bool,
) -> Result<SessionName, SessionError> {
    project
        .parse::<SessionName>()
        .map_err(|e| SessionError::Refused(e.to_string()))?;
    let base = format!("{project}_{time}");
    let mut candidate = base.clone();
    let mut n = 2;
    while exists(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
        .parse()
        .map_err(|e: crate::session_folder::InvalidSessionName| {
            SessionError::Refused(e.to_string())
        })
}
pub type OpenDriverFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Box<dyn Driver>, SessionError>> + Send + 'a>,
>;
pub struct Opening {
    pub project: String,
    pub worktree: PathBuf,
    pub settings: crate::project::Settings,
    pub name: SessionName,
}
#[derive(Default)]
pub struct Mounts {
    pub tree: PathBuf,
    pub layer: PathBuf,
}
pub trait DriverFactory: Send + Sync {
    fn prepare<'a>(
        &'a self,
        _folder: &'a SessionFolder,
        _opening: &'a Opening,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Mounts, SessionError>> + Send + 'a>,
    > {
        Box::pin(async { Ok(Mounts::default()) })
    }
    fn open<'a>(
        &'a self,
        folder: &'a SessionFolder,
        opening: &'a Opening,
        mounts: &'a Mounts,
    ) -> OpenDriverFuture<'a>;
}
pub struct Actor {
    pub project: String,
    pub worktree: PathBuf,
    pub settings: crate::project::Settings,
    pub goal: String,
    pub warnings: Vec<String>,
    pub revision: Revision,
    pub started: chrono::DateTime<chrono::Local>,
    pub checklist: Checklist,
    pub runner: Arc<dyn crate::process::ProcessRunner>,
    pub closing: Arc<AtomicBool>,
    pub name: SessionName,
    pub folder: SessionFolder,
    pub factory: Arc<dyn DriverFactory>,
    pub clock: Arc<dyn Clock + Send + Sync>,
    pub catalog: Arc<Catalog>,
    pub events: watch::Sender<SessionSnapshot>,
}
impl ActorRequest {
    pub fn into_reply(self) -> oneshot::Sender<Response> {
        match self {
            Self::Call { reply, .. }
            | Self::AddCheck { reply, .. }
            | Self::MarkCheck { reply, .. }
            | Self::Close { reply, .. } => reply,
        }
    }
}
pub enum ActorRequest {
    Call {
        tool: String,
        arguments: Value,
        reply: oneshot::Sender<Response>,
    },
    AddCheck {
        text: String,
        reply: oneshot::Sender<Response>,
    },
    MarkCheck {
        check: CheckId,
        outcome: CheckOutcome,
        comment: String,
        reply: oneshot::Sender<Response>,
    },
    Close {
        summary: String,
        reply: oneshot::Sender<Response>,
    },
}
pub struct Finished {
    pub name: SessionName,
    pub response: Response,
    pub reply: Option<oneshot::Sender<Response>>,
}
fn refusal(error: impl std::fmt::Display) -> Response {
    Response::Refused {
        message: error.to_string(),
    }
}
impl Actor {
    fn publish(&self, state: SessionState, last_action: &Option<LastAction>) {
        self.events.send_replace(SessionSnapshot {
            state,
            checks: self
                .checklist
                .checks()
                .iter()
                .map(|c| CheckStatus {
                    id: c.id,
                    text: c.text.clone(),
                    mark: c.mark.clone(),
                })
                .collect(),
            last_action: last_action.clone(),
        });
    }
    pub async fn run(
        mut self,
        mut mailbox: mpsc::Receiver<ActorRequest>,
        open_reply: oneshot::Sender<Response>,
    ) -> Finished {
        let opening = Opening {
            project: self.project.clone(),
            worktree: self.worktree.clone(),
            settings: self.settings.clone(),
            name: self.name.clone(),
        };
        let mounts = match self.factory.prepare(&self.folder, &opening).await {
            Ok(mounts) => mounts,
            Err(error) => {
                return Finished {
                    name: self.name,
                    response: refusal(error),
                    reply: Some(open_reply),
                };
            }
        };
        // Outlasts the factory's 30 s handshake limit, whose refusal names the log files.
        let mut driver = match tokio::time::timeout(
            Duration::from_secs(45),
            self.factory.open(&self.folder, &opening, &mounts),
        )
        .await
        .unwrap_or_else(|_| {
            Err(SessionError::Refused(
                "Agent Session setup timed out after 45 s. Check the session logs and retry."
                    .into(),
            ))
        }) {
            Ok(d) => d,
            Err(e) => {
                return Finished {
                    name: self.name,
                    response: refusal(e),
                    reply: Some(open_reply),
                };
            }
        };
        let opened = self.setup(driver.as_mut()).await;
        let zero = match opened {
            Ok(zero) => zero,
            Err(error) => {
                let _ = close_driver(driver.as_mut()).await;
                return Finished {
                    name: self.name,
                    response: refusal(error),
                    reply: Some(open_reply),
                };
            }
        };
        self.publish(SessionState::Open, &None);
        let _ = open_reply.send(Response::Opened {
            warnings: self.warnings.clone(),
            session: self.name.clone(),
            checks: self
                .checklist
                .checks()
                .iter()
                .map(|c| crate::ipc::OpenedCheck {
                    id: c.id,
                    text: c.text.clone(),
                })
                .collect(),
        });
        let mut timeline = SessionTimeline::new(VIDEO_LEAD);
        let mut last_action: Option<LastAction> = None;
        while let Some(request) = mailbox.recv().await {
            match request {
                ActorRequest::Call {
                    tool,
                    arguments,
                    reply,
                } => {
                    let (arguments, action) =
                        match self.catalog.prepare_call(&tool, &self.name, arguments) {
                            Ok(args) => args,
                            Err(e) => {
                                let _ = reply.send(refusal(e));
                                continue;
                            }
                        };
                    let start_ms = timeline_time(zero, self.clock.now());
                    let result =
                        call_driver(driver.as_mut(), &tool, arguments, Duration::from_secs(150))
                            .await;
                    let end_ms = timeline_time(zero, self.clock.now());
                    let is_error = result.as_ref().map_or(true, |v| {
                        v.get("isError").and_then(Value::as_bool).unwrap_or(false)
                    });
                    timeline.push(TimelineEvent::Call {
                        tool: tool.clone(),
                        action,
                        start_ms,
                        end_ms,
                        is_error,
                    });
                    if action {
                        last_action = Some(LastAction {
                            tool,
                            time_ms: start_ms,
                        });
                        self.publish(SessionState::Open, &last_action);
                    }
                    let _ = reply.send(match result {
                        Ok(value) => Response::ToolResult(value),
                        Err(error) => refusal(error),
                    });
                }
                ActorRequest::AddCheck { text, reply } => {
                    let response = match self.checklist.add(text) {
                        Ok(check) => {
                            self.publish(SessionState::Open, &last_action);
                            Response::CheckAdded { check }
                        }
                        Err(e) => refusal(e),
                    };
                    let _ = reply.send(response);
                }
                ActorRequest::MarkCheck {
                    check,
                    outcome,
                    comment,
                    reply,
                } => {
                    let time_ms = timeline_time(zero, self.clock.now());
                    let response = match self.checklist.mark(check, outcome, comment, time_ms) {
                        Ok(()) => {
                            timeline.push(TimelineEvent::CheckMark {
                                check,
                                outcome,
                                at_ms: time_ms,
                            });
                            self.publish(SessionState::Open, &last_action);
                            Response::CheckMarked { check }
                        }
                        Err(e) => refusal(e),
                    };
                    let _ = reply.send(response);
                }
                ActorRequest::Close { summary, reply } => {
                    // Everything that can refuse runs before the session stops, so the Verifier can fix it and retry.
                    let ready = match self.checklist.verdict() {
                        Ok(verdict) => crate::evidence::writable_videos_dir(self.runner.as_ref())
                            .await
                            .map(|videos| (verdict, videos))
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    };
                    let (verdict, videos) = match ready {
                        Ok(ready) => ready,
                        Err(message) => {
                            self.closing.store(false, Ordering::SeqCst);
                            let _ = reply.send(refusal(message));
                            continue;
                        }
                    };
                    self.publish(SessionState::Closing, &last_action);
                    mailbox.close();
                    while let Ok(queued) = mailbox.try_recv() {
                        let _ = queued.into_reply().send(refusal(format!(
                            "Agent Session {} is closing. Open a new Agent Session.",
                            self.name
                        )));
                    }
                    let stopped = call_driver(
                        driver.as_mut(),
                        "stop_recording",
                        json!({}),
                        Duration::from_secs(30),
                    )
                    .await;
                    let closed = close_driver(driver.as_mut()).await;
                    let written = write_records(&self.folder, &timeline, &summary).await;
                    let stop = stopped.and_then(check_result);
                    let response = match (stop, closed) {
                        (Err(stop), Err(close)) => refusal(format!("{stop}; {close}")),
                        (Err(error), Ok(())) | (Ok(()), Err(error)) => refusal(error),
                        (Ok(()), Ok(())) => match written {
                            Err(e) => refusal(e),
                            Ok(()) => {
                                let edited = crate::editor::edit(
                                    self.runner.as_ref(),
                                    &self.folder.recording(),
                                    &timeline,
                                    &self.folder.root().join("verification.mp4"),
                                    &mut |progress| {
                                        tracing::debug!(?progress, "editing Verification Video")
                                    },
                                )
                                .await;
                                match crate::session_evidence::file(
                                    &self, verdict, videos, summary, edited,
                                )
                                .await
                                {
                                    Ok(value) => Response::Closed(value),
                                    Err(message) => refusal(message),
                                }
                            }
                        },
                    };
                    return Finished {
                        name: self.name,
                        response,
                        reply: Some(reply),
                    };
                }
            }
        }
        let _ = call_driver(
            driver.as_mut(),
            "stop_recording",
            json!({}),
            Duration::from_secs(30),
        )
        .await;
        let _ = close_driver(driver.as_mut()).await;
        let _ = write_records(&self.folder, &timeline, "").await;
        Finished {
            name: self.name,
            response: Response::WatchEnded,
            reply: None,
        }
    }
    async fn setup(&self, driver: &mut dyn Driver) -> Result<Instant, SessionError> {
        for (tool, args) in cursor_setup(&self.name) {
            check_result(call_driver(driver, tool, args, Duration::from_secs(30)).await?)?;
        }
        let zero = self.clock.now();
        check_result(
            call_driver(
                driver,
                "start_recording",
                json!({"output_dir":self.folder.root(),"record_video":true}),
                Duration::from_secs(30),
            )
            .await?,
        )?;
        Ok(zero)
    }
}
async fn call_driver(
    driver: &mut dyn Driver,
    tool: &str,
    args: Value,
    limit: Duration,
) -> Result<Value, DriverError> {
    tokio::time::timeout(limit, driver.call_tool(tool, args))
        .await
        .unwrap_or_else(|_| {
            Err(DriverError::Call {
                tool: tool.into(),
                reason: format!("timed out after {} s", limit.as_secs()),
            })
        })
}
async fn close_driver(driver: &mut dyn Driver) -> Result<(), DriverError> {
    tokio::time::timeout(Duration::from_secs(31), driver.close())
        .await
        .unwrap_or_else(|_| Err(DriverError::Close("timed out after 31 s".into())))
}
fn check_result(value: Value) -> Result<(), DriverError> {
    if value.get("isError").and_then(Value::as_bool) == Some(true) {
        Err(DriverError::Call {
            tool: "Agent Session setup or recording".into(),
            reason: value.to_string(),
        })
    } else {
        Ok(())
    }
}
fn cursor_setup(name: &SessionName) -> [(&'static str, Value); 2] {
    [
        (
            "set_config",
            json!({"cursor.motion.effects.ripple":"on","cursor.badges":"minimal"}),
        ),
        (
            "set_agent_cursor_motion",
            json!({"session":name.to_string(),"effects":{"ripple":"on"},"idle_hide_ms":1000}),
        ),
    ]
}
async fn write_records(
    folder: &SessionFolder,
    timeline: &SessionTimeline,
    summary: &str,
) -> Result<(), SessionError> {
    let folder = folder.clone();
    let bytes = serde_json::to_vec_pretty(timeline)?;
    let summary = summary.to_owned();
    tokio::task::spawn_blocking(move || -> Result<(), std::io::Error> {
        crate::session_folder::write_new(&folder.root().join("timeline.json.tmp"), &bytes)?;
        std::fs::rename(folder.root().join("timeline.json.tmp"), folder.timeline())?;
        crate::session_folder::write_new(&folder.root().join("summary.txt"), summary.as_bytes())
    })
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))??;
    Ok(())
}
#[derive(Clone, serde::Serialize)]
pub struct Revision {
    pub branch: Option<String>,
    pub commit: String,
    pub dirty: bool,
}
// git runs on the host, in a folder the caller picks, and a repository's config can
// name programs. `filter.<name>.clean` can't be turned off this way.
pub(crate) const GIT_WITHOUT_REPOSITORY_PROGRAMS: [&str; 5] = [
    "--no-lazy-fetch",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.hooksPath=/dev/null",
];
async fn git(
    runner: &dyn crate::process::ProcessRunner,
    path: &Path,
    args: &[&str],
) -> Result<String, crate::process::ProcessError> {
    let arguments: Vec<&str> = GIT_WITHOUT_REPOSITORY_PROGRAMS
        .iter()
        .chain(args)
        .copied()
        .collect();
    runner.output("git", &arguments, Some(path)).await
}
pub async fn revision(
    runner: &dyn crate::process::ProcessRunner,
    path: &Path,
) -> Result<(std::path::PathBuf, Revision), SessionError> {
    let error = |e: crate::process::ProcessError| {
        SessionError::Refused(format!(
            "Cannot read git worktree {}: {e}. Pass an existing git worktree.",
            path.display()
        ))
    };
    let root = git(runner, path, &["rev-parse", "--show-toplevel"])
        .await
        .map_err(error)?;
    let branch = git(
        runner,
        path,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )
    .await
    .ok();
    let commit = git(runner, path, &["rev-parse", "HEAD"])
        .await
        .map_err(error)?;
    let dirty = !git(runner, path, &["status", "--porcelain"])
        .await
        .map_err(error)?
        .is_empty();
    Ok((
        root.into(),
        Revision {
            branch,
            commit,
            dirty,
        },
    ))
}
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_actor_tests.rs"]
mod actor_tests;
