use crate::catalog::Catalog;
use crate::checklist::Checklist;
use crate::clock::Clock;
use crate::driver::{Driver, DriverError};
use crate::ipc::{CheckStatus, LastAction, Response, SessionSnapshot, SessionState};
use crate::revision::Revision;
use crate::session_folder::{SessionFolder, SessionName};
#[cfg(test)]
use crate::timeline::{CheckId, CheckOutcome};
use crate::timeline::{Millis, SessionTimeline, TimelineEvent};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot, watch};

pub(crate) const VIDEO_LEAD: Millis = Millis(116);
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
    pub placement: crate::compositor::Placement,
    pub credentials: crate::credentials::Credentials,
    pub project: String,
    pub worktree: PathBuf,
    pub settings: crate::project::Settings,
    pub name: SessionName,
    pub progress: crate::open_progress::OpenProgress,
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
    pub frames_dir: PathBuf,
    pub journal: Option<crate::session_journal::Handle>,
    pub idle_limit: Duration,
    pub opener: Option<watch::Receiver<()>>,
    pub control: control::Control,
    pub control_commands: Option<mpsc::Receiver<control::Command>>,
    pub compositor: Arc<dyn crate::compositor::Compositor>,
    pub opened_at: Instant,
    pub placement: crate::compositor::Placement,
    pub credentials: crate::credentials::Credentials,
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
    pub progress: crate::open_progress::OpenProgress,
}
pub use crate::session_messages::{ActorRequest, CloseRequest, Finished};
fn refusal(error: impl std::fmt::Display) -> Response {
    Response::Refused {
        message: error.to_string(),
    }
}
impl Actor {
    fn mask_reply(&self, response: Response) -> Response {
        match response {
            Response::ToolResult(value) => Response::ToolResult(self.credentials.mask_value(value)),
            Response::Closed(value) => Response::Closed(self.credentials.mask_value(value)),
            Response::Refused { message } => Response::Refused {
                message: self.credentials.mask(&message),
            },
            Response::Opened { .. }
            | Response::Status(_)
            | Response::Tools(_)
            | Response::CheckAdded { .. }
            | Response::CheckMarked { .. }
            | Response::WatchEnded
            | Response::ShuttingDown
            | Response::ShutdownRefused { .. } => response,
        }
    }

    fn publish(&self, state: SessionState, last_action: &Option<LastAction>) {
        self.events.send_modify(|snapshot| {
            snapshot.state = state;
            snapshot.checks = self
                .checklist
                .checks()
                .iter()
                .map(|c| CheckStatus {
                    added: c.added,
                    id: c.id,
                    text: c.text.clone(),
                    mark: c.mark.clone(),
                })
                .collect();
            snapshot.last_action = last_action.clone();
        });
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
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_actor_tests.rs"]
mod actor_tests;

#[path = "session_gate.rs"]
pub mod gate;

#[path = "session_control.rs"]
pub mod control;

#[path = "session_run.rs"]
mod run;

#[path = "session_end.rs"]
mod end;
async fn opener_left(opener: &mut Option<watch::Receiver<()>>) {
    match opener {
        Some(receiver) => {
            let _ = receiver.changed().await;
        }
        None => std::future::pending().await,
    }
}

#[path = "session_hand_back.rs"]
mod hand_back;

#[path = "session_journal_watch.rs"]
mod journal_watch;

#[path = "session_call.rs"]
mod call;
#[path = "session_help.rs"]
mod help;

impl Actor {
    async fn opener_gone(&self, opener: &mut Option<watch::Receiver<()>>) {
        opener_left(opener).await;
        // A dropped watch sender stays ready while a queued close must finish.
        if self.closing.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
    }
}

#[path = "session_inbox.rs"]
mod inbox;
pub use inbox::Inbox;
#[path = "session_close.rs"]
mod close;
