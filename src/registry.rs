use crate::catalog::Catalog;
use crate::clock::Clock;
use crate::compositor::Compositor;
use crate::ipc::{Event, Request, Response, SessionSnapshot, SessionState};
use crate::process::ProcessRunner;
use crate::revision::revision;
use crate::session::{
    Actor, ActorRequest, DriverFactory, Finished, HostEnvironment, SessionError, choose_name,
};
use crate::session_folder::{SessionFolder, SessionName};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;

pub struct Registry {
    sessions: HashMap<SessionName, Entry>,
    pub tasks: JoinSet<Finished>,
    pub background: JoinSet<()>,
    pub preparing: JoinSet<Prepared>,
    pub catalog: watch::Receiver<Option<Result<Arc<Catalog>, String>>>,
    pub dependencies: Dependencies,
    lifecycle: Lifecycle,
}
struct Entry {
    control: crate::session::control::Control,
    slot: crate::viewer::SlotId,
    closing: Arc<AtomicBool>,
    worktree: PathBuf,
    mailbox: mpsc::Sender<ActorRequest>,
    events: watch::Receiver<SessionSnapshot>,
}
#[derive(Clone, Copy)]
enum Lifecycle {
    Running,
    ShuttingDown,
}
pub struct Dependencies {
    pub viewer: Arc<dyn crate::viewer::Viewer>,
    pub sessions_dir: PathBuf,
    pub projects_config: PathBuf,
    pub projects_cache: PathBuf,
    pub trees: PathBuf,
    pub cap: usize,
    pub host: HostEnvironment,
    pub clock: Arc<dyn Clock + Send + Sync>,
    pub runner: Arc<dyn ProcessRunner>,
    pub compositor: Arc<dyn Compositor>,
    pub factory: Arc<dyn DriverFactory>,
}
pub struct Prepared {
    result: Result<Opening, SessionError>,
    reply: oneshot::Sender<Response>,
}
struct Opening {
    credentials: crate::credentials::Credentials,
    project: String,
    worktree: PathBuf,
    goal: String,
    checks: Vec<String>,
    catalog: Arc<Catalog>,
    revision: crate::revision::Revision,
    settings: crate::project::Settings,
    warnings: Vec<String>,
}
impl Registry {
    pub fn new(
        dependencies: Dependencies,
        catalog: watch::Receiver<Option<Result<Arc<Catalog>, String>>>,
    ) -> Self {
        Self {
            sessions: HashMap::new(),
            tasks: JoinSet::new(),
            background: JoinSet::new(),
            preparing: JoinSet::new(),
            catalog,
            dependencies,
            lifecycle: Lifecycle::Running,
        }
    }
    pub fn control(&self, command: crate::viewer::ViewerCommand) {
        let (session, command) = match command {
            crate::viewer::ViewerCommand::TakeControl(session) => {
                (session, crate::session::control::Command::TakeControl)
            }
            crate::viewer::ViewerCommand::Done(session) => {
                (session, crate::session::control::Command::Done)
            }
        };
        if let Some(entry) = self.sessions.get(&session)
            && let Err(error) = entry.control.commands.try_send(command)
        {
            tracing::warn!(%error, "Takeover command could not be delivered");
        }
    }
    pub fn len(&self) -> usize {
        self.sessions.len() + self.preparing.len()
    }
    pub fn keeps_daemon_alive(&self) -> bool {
        !self.is_empty()
    }
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.preparing.is_empty()
    }
    pub fn shutdown(&mut self) -> Response {
        if self.is_empty() {
            self.lifecycle = Lifecycle::ShuttingDown;
            Response::ShuttingDown
        } else {
            Response::ShutdownRefused {
                open_sessions: self.len(),
            }
        }
    }
    pub async fn finished(&mut self, finished: Finished) {
        if let Some(entry) = self.sessions.remove(&finished.name) {
            self.dependencies.viewer.closed(entry.slot).await;
        }
        let tree = crate::session_project::tree_path(&self.dependencies.trees, &finished.name);
        let cleanup = tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(tree) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        })
        .await;
        match cleanup {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "could not remove the worktree's copy"),
            Err(error) => tracing::error!(%error, "could not finish removing the worktree's copy"),
        }
        if self.sessions.is_empty()
            && let Err(error) = self.dependencies.compositor.remove().await
        {
            tracing::error!(%error,"could not disable Session Window rule");
        }
        if let Some(reply) = finished.reply {
            let _ = reply.send(finished.response);
        }
    }
    pub async fn handle(
        &mut self,
        request: Request,
        reply: oneshot::Sender<Response>,
        events: Option<mpsc::Sender<Event>>,
    ) {
        match request {
            Request::OpenSession {
                project,
                worktree,
                goal,
                checks,
            } => {
                self.begin_open(project, worktree, goal, checks, reply);
            }
            Request::CallTool {
                session,
                tool,
                arguments,
            } => {
                if let Some(entry) = self.sessions.get_mut(&session) {
                    if entry.closing.load(Ordering::SeqCst) {
                        let _ = reply.send(Response::Refused {
                            message: format!(
                                "Agent Session {session} is closing. Open a new Agent Session."
                            ),
                        });
                        return;
                    }
                    send_actor(
                        &entry.mailbox,
                        ActorRequest::Call {
                            arrival: entry.control.gate.arrival(),
                            tool,
                            arguments,
                            reply,
                        },
                        &session,
                    );
                } else {
                    let _ = reply.send(self.unknown(&session));
                }
            }
            Request::RequestHelp { session, reason } => {
                if let Some(entry) = self.sessions.get(&session) {
                    send_actor(
                        &entry.mailbox,
                        ActorRequest::RequestHelp {
                            arrival: entry.control.gate.arrival(),
                            reason,
                            reply,
                        },
                        &session,
                    );
                } else {
                    let _ = reply.send(self.unknown(&session));
                }
            }
            Request::AddCheck { session, text } => {
                self.send_check(&session, ActorRequest::AddCheck { text, reply });
            }
            Request::MarkCheck {
                session,
                check,
                outcome,
                comment,
            } => {
                self.send_check(
                    &session,
                    ActorRequest::MarkCheck {
                        check,
                        outcome,
                        comment,
                        reply,
                    },
                );
            }
            Request::CloseSession { session, summary } => {
                if let Some(entry) = self.sessions.get_mut(&session) {
                    if entry.closing.swap(true, Ordering::SeqCst) {
                        let _ = reply.send(Response::Refused { message: format!("Agent Session {session} is already closing. Open a new Agent Session.") });
                        return;
                    }
                    if !send_actor(
                        &entry.mailbox,
                        ActorRequest::Close { summary, reply },
                        &session,
                    ) {
                        entry.closing.store(false, Ordering::SeqCst);
                    }
                } else {
                    let _ = reply.send(self.unknown(&session));
                }
            }
            Request::WatchSession { session } => {
                if let Some(entry) = self.sessions.get(&session) {
                    let mut receiver = entry.events.clone();
                    self.background.spawn(async move {
                        if let Some(events) = events {
                            loop {
                                let event = Event::Snapshot(receiver.borrow_and_update().clone());
                                if events.send(event).await.is_err() {
                                    return;
                                }
                                if receiver.changed().await.is_err() {
                                    break;
                                }
                            }
                        }
                        let _ = reply.send(Response::WatchEnded);
                    });
                } else {
                    let _ = reply.send(self.unknown(&session));
                }
            }
            Request::ListTools => {
                let mut catalog = self.catalog.clone();
                self.background.spawn(async move {
                    let result = wait_catalog(&mut catalog).await;
                    let tools = result.map_or_else(|_| vec![], |c| c.tools());
                    let _ = reply.send(Response::Tools(serde_json::json!({"tools":tools})));
                });
            }
            Request::Status => {
                let _ = reply.send(Response::Status(crate::ipc::DaemonStatus {
                    version: crate::VERSION.into(),
                    pid: std::process::id(),
                    open_sessions: self.len(),
                }));
            }
            Request::Shutdown => {
                let response = self.shutdown();
                let _ = reply.send(response);
            }
        }
    }
    fn unknown(&self, name: &SessionName) -> Response {
        let mut names: Vec<_> = self.sessions.keys().map(ToString::to_string).collect();
        names.sort();
        Response::Refused {
            message: format!(
                "Unknown Agent Session {name}. Open Agent Sessions: {}. Use an open session name or call open_session.",
                if names.is_empty() {
                    "none".to_owned()
                } else {
                    names.join(", ")
                }
            ),
        }
    }
    fn send_check(&self, session: &SessionName, request: ActorRequest) {
        // A close queued ahead may yet be refused, so the actor decides in mailbox order.
        if let Some(entry) = self.sessions.get(session) {
            send_actor(&entry.mailbox, request, session);
        } else {
            let _ = request.into_reply().send(self.unknown(session));
        }
    }
    pub async fn prepared(&mut self, prepared: Prepared) {
        let Prepared { result, reply } = prepared;
        let opened = match result {
            Ok(opening) => self.claim(opening).await,
            Err(error) => Err(error),
        };
        match opened {
            Ok((actor, mailbox)) => {
                self.tasks.spawn(actor.run(mailbox, reply));
            }
            Err(error) => {
                let _ = reply.send(Response::Refused {
                    message: error.to_string(),
                });
            }
        }
    }
}
pub(crate) fn send_actor(
    mailbox: &mpsc::Sender<ActorRequest>,
    request: ActorRequest,
    session: &SessionName,
) -> bool {
    match mailbox.try_send(request) {
        Ok(()) => true,
        Err(error) => {
            let (request, reason) = match error {
                mpsc::error::TrySendError::Full(request) => (request, "is busy"),
                mpsc::error::TrySendError::Closed(request) => (request, "closed"),
            };
            let _ = request.into_reply().send(Response::Refused {
                message: format!(
                    "Agent Session {session} {reason}. Try again or open a new Agent Session."
                ),
            });
            false
        }
    }
}
pub async fn wait_catalog(
    receiver: &mut watch::Receiver<Option<Result<Arc<Catalog>, String>>>,
) -> Result<Arc<Catalog>, String> {
    tokio::time::timeout(std::time::Duration::from_secs(30),async {
        loop {if let Some(result)=receiver.borrow_and_update().clone(){return result;}receiver.changed().await.map_err(|e|format!("Cua Driver catalog unavailable: {e}. Restart keek."))?;}
    }).await.map_err(|_|"Cua Driver catalog did not become ready within 30 s. Check the pinned driver installation and restart keek.".to_owned())?
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

#[path = "registry_open.rs"]
mod opening;
