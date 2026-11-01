use crate::catalog::Catalog;
use crate::clock::Clock;
use crate::compositor::Compositor;
use crate::ipc::{Event, Request, Response, SessionSnapshot, SessionState};
use crate::process::ProcessRunner;
use crate::session::{
    Actor, ActorRequest, DriverFactory, Finished, HostEnvironment, SessionError, choose_name,
    revision,
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
    pub sessions_dir: PathBuf,
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
    project: String,
    worktree: PathBuf,
    goal: String,
    checks: Vec<String>,
    catalog: Arc<Catalog>,
    revision: crate::session::Revision,
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
        self.sessions.remove(&finished.name);
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
    fn begin_open(
        &mut self,
        project: String,
        worktree: PathBuf,
        goal: String,
        checks: Vec<String>,
        reply: oneshot::Sender<Response>,
    ) {
        let refusal = match self.lifecycle {
            Lifecycle::ShuttingDown => {
                Some("keek is ShuttingDown. Start keek again to open an Agent Session.".to_owned())
            }
            Lifecycle::Running if self.len() >= self.dependencies.cap => Some(format!(
                "Agent Session cap {} reached. Close an open Agent Session first.",
                self.dependencies.cap
            )),
            Lifecycle::Running => None,
        };
        if let Some(message) = refusal {
            let _ = reply.send(Response::Refused { message });
            return;
        }
        let host = self.dependencies.host.clone();
        let runner = Arc::clone(&self.dependencies.runner);
        let sessions_dir = self.dependencies.sessions_dir.clone();
        let mut catalog = self.catalog.clone();
        self.preparing.spawn(async move {
            let result = async {
                host.validate()?;
                project
                    .parse::<SessionName>()
                    .map_err(|e| SessionError::Refused(e.to_string()))?;
                let catalog = wait_catalog(&mut catalog)
                    .await
                    .map_err(SessionError::Refused)?;
                // The daemon's working folder is /, so a relative path would name the wrong folder.
                if !worktree.is_absolute() {
                    return Err(SessionError::Refused(
                        "worktree must be an absolute path. Pass the full path to a git worktree."
                            .into(),
                    ));
                }
                let worktree = tokio::task::spawn_blocking(move || std::fs::canonicalize(worktree))
                    .await
                    .map_err(|e| SessionError::Refused(e.to_string()))?
                    .map_err(|e| {
                        SessionError::Refused(format!(
                            "Cannot open worktree: {e}. Pass an existing git worktree."
                        ))
                    })?;
                let sessions = tokio::task::spawn_blocking(move || std::fs::canonicalize(sessions_dir)).await.map_err(|e| SessionError::Refused(e.to_string()))?;
                match &sessions {
                    Ok(sessions) if worktree.starts_with(sessions) => return Err(SessionError::Refused("Cannot open a worktree inside the sessions folder. Choose a worktree outside it.".into())),
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(SessionError::Refused(format!("Cannot inspect sessions folder: {error}. Check its permissions."))),
                    _ => {}
                }
                let (root, revision) = revision(runner.as_ref(), &worktree).await?;
                let worktree = tokio::task::spawn_blocking(move || std::fs::canonicalize(root)).await.map_err(|e| SessionError::Refused(e.to_string()))?.map_err(|e| SessionError::Refused(format!("Cannot inspect git worktree root: {e}. Check the worktree and retry.")))?;
                if sessions.as_ref().is_ok_and(|sessions| worktree.starts_with(sessions)) { return Err(SessionError::Refused("Cannot open a worktree inside the sessions folder. Choose a worktree outside it.".into())); }
                Ok(Opening {
                    project,
                    worktree,
                    goal,
                    checks,
                    catalog,
                    revision,
                })
            }
            .await;
            Prepared { result, reply }
        });
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
    async fn claim(
        &mut self,
        opening: Opening,
    ) -> Result<(Actor, mpsc::Receiver<ActorRequest>), SessionError> {
        let Opening {
            project,
            worktree,
            goal,
            checks,
            catalog,
            revision,
        } = opening;
        match self.lifecycle {
            Lifecycle::Running => {}
            Lifecycle::ShuttingDown => {
                return Err(SessionError::Refused(
                    "keek is ShuttingDown. Start keek again to open an Agent Session.".into(),
                ));
            }
        }
        if self
            .sessions
            .values()
            .any(|entry| entry.worktree == worktree)
        {
            return Err(SessionError::Refused(format!(
                "An Agent Session is already open for {}. Close it first.",
                worktree.display()
            )));
        }
        let started = self.dependencies.clock.local_time();
        let time = started.format("%H-%M-%S").to_string();
        let naming_project = project.clone();
        let sessions = self.dependencies.sessions_dir.clone();
        let used: Vec<_> = self.sessions.keys().map(ToString::to_string).collect();
        let name = tokio::task::spawn_blocking(move || {
            choose_name(&naming_project, &time, |name| {
                used.iter().any(|used| used == name) || sessions.join(name).exists()
            })
        })
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))??;
        let folder = SessionFolder::new(&self.dependencies.sessions_dir, &name);
        let root = folder.root().to_owned();
        let record = serde_json::json!({"goal":goal,"checks":checks,"revision":revision,"worktree":worktree});
        tokio::task::spawn_blocking(move || -> Result<(), std::io::Error> {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&root)?;
            std::fs::write(
                root.join("session.json"),
                serde_json::to_vec_pretty(&record)?,
            )
        })
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))??;
        if self.sessions.is_empty() {
            self.dependencies
                .compositor
                .install()
                .await
                .map_err(|e| SessionError::Refused(e.to_string()))?;
        }
        let (sender, receiver) = mpsc::channel(32);
        let (events, event_receiver) = watch::channel(SessionSnapshot {
            state: SessionState::Opening,
            checks: vec![],
            last_action: None,
        });
        // Set when a close is queued, before the actor can report it, and cleared if the actor refuses it.
        let closing = Arc::new(AtomicBool::new(false));
        self.sessions.insert(
            name.clone(),
            Entry {
                closing: Arc::clone(&closing),
                worktree,
                mailbox: sender,
                events: event_receiver,
            },
        );
        Ok((
            Actor {
                project,
                goal,
                revision,
                started,
                checklist: crate::checklist::Checklist::new(checks),
                runner: Arc::clone(&self.dependencies.runner),
                closing,
                name,
                folder,
                factory: Arc::clone(&self.dependencies.factory),
                clock: Arc::clone(&self.dependencies.clock),
                catalog,
                events,
            },
            receiver,
        ))
    }
}
fn send_actor(
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
