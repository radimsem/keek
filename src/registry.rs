use crate::catalog::Catalog;
use crate::checklist::SessionEnd;
use crate::clock::Clock;
use crate::compositor::Compositor;
use crate::ipc::{Event, Request, Response, SessionSnapshot, SessionState};
use crate::process::ProcessRunner;
use crate::revision::revision;
use crate::session::{
    Actor, ActorRequest, CloseRequest, DriverFactory, Finished, HostEnvironment, Inbox,
    SessionError, choose_name,
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
    cards: HashMap<SessionName, EndedCard>,
    pub tasks: JoinSet<Finished>,
    pub background: JoinSet<()>,
    pub recovery: JoinSet<()>,
    pub preparing: JoinSet<Prepared>,
    // A finished build keeps its entry until the next prepare_project for that Project replaces it.
    prepares: HashMap<String, (PathBuf, watch::Receiver<Option<Response>>)>,
    pub catalog: watch::Receiver<Option<Result<Arc<Catalog>, String>>>,
    pub dependencies: Dependencies,
    lifecycle: Lifecycle,
}
struct EndedCard {
    slot: crate::viewer::SlotId,
    frame: Option<PathBuf>,
    end: SessionEnd,
    folder: Option<PathBuf>,
    ended: std::time::Instant,
}
struct Entry {
    card: CardDisposition,
    control: crate::session::control::Control,
    slot: crate::viewer::SlotId,
    closing: Arc<AtomicBool>,
    worktree: PathBuf,
    mailbox: mpsc::Sender<ActorRequest>,
    close: mpsc::Sender<CloseRequest>,
    events: watch::Receiver<SessionSnapshot>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum CardDisposition {
    Keep,
    Dismiss,
}
#[derive(Clone, Copy)]
enum Lifecycle {
    Running,
    ShuttingDown,
}
pub struct Dependencies {
    pub frames_dir: PathBuf,
    pub journals_dir: PathBuf,
    pub session_idle_limit: std::time::Duration,
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
    opener: Option<watch::Receiver<()>>,
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
    profile: crate::project::Profile,
    warnings: Vec<String>,
    progress: crate::open_progress::OpenProgress,
}
impl Registry {
    pub fn new(
        dependencies: Dependencies,
        catalog: watch::Receiver<Option<Result<Arc<Catalog>, String>>>,
    ) -> Self {
        Self {
            sessions: HashMap::new(),
            cards: HashMap::new(),
            tasks: JoinSet::new(),
            background: JoinSet::new(),
            recovery: JoinSet::new(),
            preparing: JoinSet::new(),
            prepares: HashMap::new(),
            catalog,
            dependencies,
            lifecycle: Lifecycle::Running,
        }
    }
    pub async fn remove_host_rule(&self) {
        if let Err(error) = self.dependencies.compositor.remove().await {
            tracing::warn!(%error, "could not remove the host window rule");
        }
    }
    pub async fn control(&mut self, command: crate::viewer::ViewerCommand) {
        let (session, command) = match command {
            crate::viewer::ViewerCommand::Dismiss(session) => {
                if let Some(card) = self.cards.remove(&session) {
                    self.dependencies.viewer.closed(card.slot).await;
                    remove_frame(card.frame).await;
                } else if let Some(entry) = self.sessions.get_mut(&session)
                    && matches!(entry.events.borrow().state, SessionState::Ended(_))
                {
                    entry.card = CardDisposition::Dismiss;
                }
                return;
            }
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
        !self.is_empty() || !self.cards.is_empty() || !self.recovery.is_empty()
    }
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.preparing.is_empty()
    }
    pub fn shutdown(&mut self) -> Response {
        if self.is_empty() && self.recovery.is_empty() {
            self.lifecycle = Lifecycle::ShuttingDown;
            Response::ShuttingDown
        } else {
            Response::ShutdownRefused {
                open_sessions: self.len() + self.recovery.len(),
            }
        }
    }
    pub async fn finished(&mut self, finished: Finished) {
        if let Some(entry) = self.sessions.remove(&finished.name) {
            let ended = match &entry.events.borrow().state {
                SessionState::Ended(ended) => {
                    Some((ended.frame.clone(), ended.end.clone(), ended.folder.clone()))
                }
                SessionState::Opening
                | SessionState::Open
                | SessionState::Closing
                | SessionState::Rendering { .. } => None,
            };
            if let Some((frame, end, folder)) = ended {
                if self.dependencies.viewer.supports_cards() && entry.card == CardDisposition::Keep
                {
                    self.cards.insert(
                        finished.name.clone(),
                        EndedCard {
                            slot: entry.slot,
                            frame,
                            end,
                            folder,
                            ended: self.dependencies.clock.now(),
                        },
                    );
                } else {
                    self.dependencies.viewer.closed(entry.slot).await;
                    remove_frame(frame).await;
                }
            } else {
                self.dependencies.viewer.closed(entry.slot).await;
            }
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
        self.handle_connection(None, request, reply, events).await;
    }
    pub async fn handle_connection(
        &mut self,
        opener: Option<watch::Receiver<()>>,
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
                self.begin_open(
                    project,
                    worktree,
                    goal,
                    checks,
                    reply,
                    opening::Caller { opener, events },
                );
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
                    if !send_close(&entry.close, CloseRequest { summary, reply }, &session) {
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
                    build: crate::build_id::current()
                        .await
                        .map(|build| build.to_string()),
                    pid: std::process::id(),
                    open_sessions: self.len(),
                }));
            }
            Request::SetProjectProfile {
                project,
                profile,
                credentials,
            } => self.set_project_profile(project, profile, credentials, reply),
            Request::PrepareProject { project, worktree } => {
                self.prepare_project(project, worktree, reply, opening::Caller { opener, events })
            }
            Request::Shutdown => {
                let response = self.shutdown();
                let _ = reply.send(response);
            }
        }
    }
    fn unknown(&self, name: &SessionName) -> Response {
        match self
            .cards
            .get(name)
            .map(|card| (&card.end, card.folder.as_deref()))
        {
            Some((SessionEnd::Incomplete(reason), folder)) => {
                return Response::Refused {
                    message: reason.ended_refusal(name, folder),
                };
            }
            Some((SessionEnd::Verdict(_), _)) | None => {}
        }
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
        // The actor applies queued checks before it decides a close.
        if let Some(entry) = self.sessions.get(session) {
            send_actor(&entry.mailbox, request, session);
        } else {
            let _ = request.into_reply().send(self.unknown(session));
        }
    }
    pub async fn prepared(&mut self, prepared: Prepared) {
        let Prepared {
            result,
            reply,
            opener,
        } = prepared;
        let opened = match result {
            Ok(opening) => self.claim(opening, opener).await,
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
    try_deliver(mailbox, request, session, ActorRequest::into_reply)
}
pub(crate) fn send_close(
    mailbox: &mpsc::Sender<CloseRequest>,
    request: CloseRequest,
    session: &SessionName,
) -> bool {
    try_deliver(mailbox, request, session, |request| request.reply)
}
fn try_deliver<T>(
    sender: &mpsc::Sender<T>,
    request: T,
    session: &SessionName,
    reply: impl FnOnce(T) -> oneshot::Sender<Response>,
) -> bool {
    match sender.try_send(request) {
        Ok(()) => true,
        Err(error) => {
            let (request, reason) = match error {
                mpsc::error::TrySendError::Full(request) => (request, "is busy"),
                mpsc::error::TrySendError::Closed(request) => (request, "closed"),
            };
            let _ = reply(request).send(Response::Refused {
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

#[path = "session_recovery.rs"]
mod recovery;

#[path = "registry_project.rs"]
mod project;

async fn remove_frame(frame: Option<PathBuf>) {
    if let Some(frame) = frame {
        let result = tokio::task::spawn_blocking(move || std::fs::remove_file(frame)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(Err(error)) => tracing::warn!(%error, "could not remove Session Card frame"),
            Err(error) => tracing::warn!(%error, "could not finish removing Session Card frame"),
        }
    }
}
