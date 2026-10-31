//! The per-user daemon's lifecycle: one instance through a lock file, one owner
//! loop over its state, and exit once nothing has needed it for the idle limit.

use std::fs::{DirBuilder, File, TryLockError};
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::time::{Duration, Instant};

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

use crate::VERSION;
use crate::clock::Clock;
use crate::ipc::{Connection, DaemonStatus, IpcError, Message, Request, Response};
use crate::paths::Paths;

pub const IDLE_LIMIT_ENV: &str = "KEEK_DAEMON_IDLE_MS";
const DEFAULT_IDLE_LIMIT: Duration = Duration::from_secs(60);
const REQUEST_QUEUE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Exit {
    Idle,
    ShutDown,
    /// Another daemon holds the lock, so this one never served.
    AlreadyRunning,
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("{IDLE_LIMIT_ENV} must be a whole number of milliseconds, not {0:?}")]
    InvalidIdleLimit(String),
    #[error("could not create the runtime folder")]
    RuntimeDir(#[source] io::Error),
    #[error("could not take the daemon lock")]
    Lock(#[source] io::Error),
    #[error("could not bind the daemon socket")]
    Bind(#[source] io::Error),
    #[error("could not accept a client")]
    Accept(#[source] io::Error),
    #[error("could not remove the daemon socket")]
    RemoveSocket(#[source] io::Error),
}

/// Reads the idle limit from the value of [`IDLE_LIMIT_ENV`].
pub fn idle_limit(value: Option<&str>) -> Result<Duration, DaemonError> {
    let Some(value) = value else {
        return Ok(DEFAULT_IDLE_LIMIT);
    };
    value
        .parse()
        .map(Duration::from_millis)
        .map_err(|_| DaemonError::InvalidIdleLimit(value.to_owned()))
}

pub async fn run(
    paths: &Paths,
    idle_limit: Duration,
    clock: impl Clock,
) -> Result<Exit, DaemonError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(paths.runtime_dir())
        .map_err(DaemonError::RuntimeDir)?;

    // The kernel releases this lock when the process ends, however it ends, so a
    // socket left by a crash is safe to replace once the lock is held.
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.lock())
        .map_err(DaemonError::Lock)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(Exit::AlreadyRunning),
        Err(TryLockError::Error(error)) => return Err(DaemonError::Lock(error)),
    }

    let socket = paths.socket();
    remove_socket(&socket)?;
    let listener = UnixListener::bind(&socket).map_err(DaemonError::Bind)?;

    let exit = serve(listener, IdleWatch::new(clock, idle_limit)).await;
    let removed = remove_socket(&socket);
    let exit = exit?;
    removed?;
    Ok(exit)
}

fn remove_socket(socket: &Path) -> Result<(), DaemonError> {
    match std::fs::remove_file(socket) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            Err(DaemonError::RemoveSocket(error))
        }
        Ok(()) | Err(_) => Ok(()),
    }
}

async fn serve(
    listener: UnixListener,
    mut idle: IdleWatch<impl Clock>,
) -> Result<Exit, DaemonError> {
    let state = State::default();
    let (request_sender, mut requests) = mpsc::channel::<PendingRequest>(REQUEST_QUEUE);
    let mut connections: JoinSet<ConnectionEnd> = JoinSet::new();

    loop {
        idle.update(state.keeps_daemon_alive() || !connections.is_empty());
        let remaining = idle.remaining();

        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(DaemonError::Accept)?;
                connections.spawn(serve_client(stream, request_sender.clone()));
            }
            Some(ended) = connections.join_next() => {
                match ended {
                    Ok(ConnectionEnd::ShutdownAccepted) => return Ok(Exit::ShutDown),
                    Ok(ConnectionEnd::ClientLeft) => {}
                    Err(error) => tracing::error!(%error, "a client connection task failed"),
                }
            }
            Some(pending) = requests.recv() => {
                // The client may have gone, and then nobody needs the answer.
                let _ = pending.reply.send(state.handle(&pending.request));
            }
            () = tokio::time::sleep(remaining.unwrap_or_default()), if remaining.is_some() => {
                return Ok(Exit::Idle);
            }
        }
    }
}

#[derive(Debug, Default)]
struct State {
    open_sessions: usize,
    session_cards: usize,
}

impl State {
    fn keeps_daemon_alive(&self) -> bool {
        self.open_sessions > 0 || self.session_cards > 0
    }

    fn handle(&self, request: &Request) -> Response {
        match request {
            Request::Status => Response::Status(DaemonStatus {
                version: VERSION.to_owned(),
                pid: std::process::id(),
                open_sessions: self.open_sessions,
            }),
            Request::Shutdown if self.open_sessions > 0 => Response::ShutdownRefused {
                open_sessions: self.open_sessions,
            },
            Request::Shutdown => Response::ShuttingDown,
        }
    }
}

struct PendingRequest {
    request: Request,
    reply: oneshot::Sender<Response>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionEnd {
    ClientLeft,
    ShutdownAccepted,
}

async fn serve_client(stream: UnixStream, requests: mpsc::Sender<PendingRequest>) -> ConnectionEnd {
    serve_connection(Connection::new(stream), requests)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "dropping a client");
            ConnectionEnd::ClientLeft
        })
}

#[derive(Debug, thiserror::Error)]
enum ConnectionError {
    #[error(transparent)]
    Ipc(#[from] IpcError),
    #[error("the client broke the protocol by sending {0:?}")]
    Protocol(Box<Message>),
    #[error("the daemon's owner loop stopped")]
    OwnerGone,
}

async fn serve_connection<S>(
    mut connection: Connection<S>,
    requests: mpsc::Sender<PendingRequest>,
) -> Result<ConnectionEnd, ConnectionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match connection.recv().await? {
        None => return Ok(ConnectionEnd::ClientLeft),
        Some(Message::Hello { version }) => {
            tracing::info!(client_version = %version, "client connected");
        }
        Some(other) => return Err(ConnectionError::Protocol(Box::new(other))),
    }
    connection
        .send(&Message::Hello {
            version: VERSION.to_owned(),
        })
        .await?;

    loop {
        let (id, request) = match connection.recv().await? {
            None => return Ok(ConnectionEnd::ClientLeft),
            Some(Message::Request { id, request }) => (id, request),
            Some(other) => return Err(ConnectionError::Protocol(Box::new(other))),
        };
        let (reply, answer) = oneshot::channel();
        requests
            .send(PendingRequest { request, reply })
            .await
            .map_err(|_| ConnectionError::OwnerGone)?;
        let response = answer.await.map_err(|_| ConnectionError::OwnerGone)?;
        let shutting_down = response == Response::ShuttingDown;
        connection.send(&Message::Response { id, response }).await?;
        if shutting_down {
            return Ok(ConnectionEnd::ShutdownAccepted);
        }
    }
}

#[derive(Debug)]
struct IdleWatch<C> {
    clock: C,
    limit: Duration,
    idle_since: Option<Instant>,
}

impl<C: Clock> IdleWatch<C> {
    fn new(clock: C, limit: Duration) -> Self {
        Self {
            clock,
            limit,
            idle_since: None,
        }
    }

    fn update(&mut self, in_use: bool) {
        if in_use {
            self.idle_since = None;
        } else if self.idle_since.is_none() {
            self.idle_since = Some(self.clock.now());
        }
    }

    /// `None` while the daemon is in use.
    fn remaining(&self) -> Option<Duration> {
        let elapsed = self.clock.now().saturating_duration_since(self.idle_since?);
        Some(self.limit.saturating_sub(elapsed))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use tokio::io::duplex;

    use super::*;
    use crate::ipc::RequestId;

    const LIMIT: Duration = Duration::from_secs(60);

    struct FakeClock<'a>(&'a Cell<Instant>);

    impl Clock for FakeClock<'_> {
        fn now(&self) -> Instant {
            self.0.get()
        }
    }

    fn advance(now: &Cell<Instant>, by: Duration) {
        now.set(now.get() + by);
    }

    #[test]
    fn the_idle_limit_counts_from_when_the_daemon_stopped_being_used() {
        let now = Cell::new(Instant::now());
        let mut idle = IdleWatch::new(FakeClock(&now), LIMIT);

        idle.update(true);
        assert_eq!(idle.remaining(), None);

        idle.update(false);
        advance(&now, Duration::from_secs(45));
        idle.update(false);
        assert_eq!(idle.remaining(), Some(Duration::from_secs(15)));

        advance(&now, Duration::from_secs(20));
        assert_eq!(idle.remaining(), Some(Duration::ZERO));
    }

    #[test]
    fn being_used_again_restarts_the_idle_limit() {
        let now = Cell::new(Instant::now());
        let mut idle = IdleWatch::new(FakeClock(&now), LIMIT);

        idle.update(false);
        advance(&now, Duration::from_secs(59));
        idle.update(true);
        advance(&now, Duration::from_secs(10));
        idle.update(false);

        assert_eq!(idle.remaining(), Some(LIMIT));
    }

    #[test]
    fn open_sessions_and_session_cards_keep_the_daemon_alive() {
        assert!(!State::default().keeps_daemon_alive());
        assert!(
            State {
                open_sessions: 1,
                session_cards: 0
            }
            .keeps_daemon_alive()
        );
        assert!(
            State {
                open_sessions: 0,
                session_cards: 1
            }
            .keeps_daemon_alive()
        );
    }

    #[test]
    fn shutdown_is_refused_only_while_an_agent_session_is_open() {
        let with_session = State {
            open_sessions: 2,
            session_cards: 0,
        };
        let with_card_only = State {
            open_sessions: 0,
            session_cards: 1,
        };

        assert_eq!(
            with_session.handle(&Request::Shutdown),
            Response::ShutdownRefused { open_sessions: 2 }
        );
        assert_eq!(
            with_card_only.handle(&Request::Shutdown),
            Response::ShuttingDown
        );
    }

    #[test]
    fn the_idle_limit_defaults_to_60_seconds_and_refuses_nonsense() {
        assert_eq!(idle_limit(None).unwrap(), Duration::from_secs(60));
        assert_eq!(idle_limit(Some("250")).unwrap(), Duration::from_millis(250));
        assert!(matches!(
            idle_limit(Some("soon")),
            Err(DaemonError::InvalidIdleLimit(_))
        ));
    }

    #[tokio::test]
    async fn a_client_that_skips_hello_is_dropped() {
        let (near, far) = duplex(1024);
        let (sender, _requests) = mpsc::channel(1);
        let mut client = Connection::new(near);

        client
            .send(&Message::Request {
                id: RequestId(1),
                request: Request::Status,
            })
            .await
            .unwrap();
        let result = serve_connection(Connection::new(far), sender).await;

        assert!(matches!(result, Err(ConnectionError::Protocol(_))));
    }
}
