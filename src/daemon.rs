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
use crate::ipc::{Connection, IpcError, Message, Request, Response};
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
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
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
    clock: impl Clock + Send + Sync + 'static,
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

    let config = crate::config::Config::read(paths.config_file())
        .await
        .map_err(DaemonError::Config)?;
    let runner: std::sync::Arc<dyn crate::process::ProcessRunner> =
        std::sync::Arc::new(crate::process::HostRunner);
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let dependencies = crate::registry::Dependencies {
        sessions_dir: paths.sessions_dir().to_owned(),
        cap: config.session_cap,
        host: crate::session::HostEnvironment::capture(),
        clock: std::sync::Arc::new(clock),
        runner: runner.clone(),
        compositor: std::sync::Arc::new(crate::compositor::HostCompositor::new(runner.clone())),
        factory: std::sync::Arc::new(crate::session_factory::SandboxFactory {
            driver: crate::driver::driver_path(&home),
            runner,
            start_command: std::env::var("KEEK_TEST_START_COMMAND").ok(),
        }),
    };
    let (catalog_sender, catalog_receiver) = tokio::sync::watch::channel(None);
    let mut registry = crate::registry::Registry::new(dependencies, catalog_receiver);
    let runtime = paths.runtime_dir().to_owned();
    registry.background.spawn(async move {
        let result = crate::catalog::load(&runtime, &home)
            .await
            .map(std::sync::Arc::new)
            .map_err(|error| error.to_string());
        catalog_sender.send_replace(Some(result));
    });
    let exit = serve(
        listener,
        IdleWatch::new(crate::clock::MonotonicClock, idle_limit),
        registry,
    )
    .await;
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
    mut registry: crate::registry::Registry,
) -> Result<Exit, DaemonError> {
    let (request_sender, mut requests) = mpsc::channel::<PendingRequest>(REQUEST_QUEUE);
    let mut connections: JoinSet<ConnectionEnd> = JoinSet::new();

    loop {
        idle.update(registry.keeps_daemon_alive() || !connections.is_empty());
        let remaining = idle.remaining();

        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => { connections.spawn(serve_client(stream, request_sender.clone())); }
                    Err(error) => { tracing::warn!(%error, "could not accept a client; retrying"); tokio::time::sleep(Duration::from_millis(50)).await; }
                }
            }
            Some(prepared) = registry.preparing.join_next() => {
                match prepared { Ok(prepared) => registry.prepared(prepared).await, Err(error) => tracing::error!(%error, "Agent Session preparation failed") }
            }
            Some(finished) = registry.tasks.join_next() => {
                match finished { Ok(finished) => registry.finished(finished).await, Err(error) => tracing::error!(%error, "Agent Session task failed") }
            }
            Some(ended) = registry.background.join_next() => { if let Err(error) = ended { tracing::error!(%error, "daemon background task failed"); } }
            Some(ended) = connections.join_next() => {
                match ended {
                    Ok(ConnectionEnd::ShutdownAccepted) => return Ok(Exit::ShutDown),
                    Ok(ConnectionEnd::ClientLeft) => {}
                    Err(error) => tracing::error!(%error, "a client connection task failed"),
                }
            }
            Some(pending) = requests.recv() => {
                registry.handle(pending.request, pending.reply, pending.events).await;
            }
            () = tokio::time::sleep(remaining.unwrap_or_default()), if remaining.is_some() => {
                return Ok(Exit::Idle);
            }
        }
    }
}

struct PendingRequest {
    request: Request,
    reply: oneshot::Sender<Response>,
    events: Option<mpsc::Sender<crate::ipc::Event>>,
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
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    match connection.recv().await? {
        None => return Ok(ConnectionEnd::ClientLeft),
        Some(Message::Hello { version }) => {
            tracing::info!(client_version = %version, "client connected")
        }
        Some(other) => return Err(ConnectionError::Protocol(Box::new(other))),
    }
    connection
        .send(&Message::Hello {
            version: VERSION.to_owned(),
        })
        .await?;
    let (mut reader, mut writer) = connection.split();
    let (incoming, mut messages) = mpsc::channel(REQUEST_QUEUE);
    let (outgoing, mut replies) = mpsc::channel(REQUEST_QUEUE);
    let mut tasks = JoinSet::<Result<(), ConnectionError>>::new();
    tasks.spawn(async move {
        loop {
            let message = reader.recv().await;
            let ended = !matches!(message, Ok(Some(_)));
            if incoming.send(message).await.is_err() || ended {
                return Ok(());
            }
        }
    });
    loop {
        tokio::select! {
            message = messages.recv() => {
                let (id, request) = match message {
                    None | Some(Ok(None)) => return Ok(ConnectionEnd::ClientLeft),
                    Some(Err(error)) => return Err(error.into()),
                    Some(Ok(Some(Message::Request { id, request }))) => (id, request),
                    Some(Ok(Some(other))) => return Err(ConnectionError::Protocol(Box::new(other))),
                };
                let (reply, mut answer) = oneshot::channel();
                let (events, mut event_receiver) = mpsc::channel(REQUEST_QUEUE);
                let watches = matches!(request, Request::WatchSession { .. });
                requests.send(PendingRequest { request, reply, events: watches.then_some(events) }).await.map_err(|_| ConnectionError::OwnerGone)?;
                let outgoing = outgoing.clone();
                tasks.spawn(async move {
                    loop {
                        tokio::select! {
                            response = &mut answer => {
                                let response = response.unwrap_or_else(|_| Response::Refused { message: "The Agent Session ended before it answered. Open a new Agent Session.".into() });
                                outgoing.send(Message::Response { id, response }).await.map_err(|_| ConnectionError::OwnerGone)?;
                                return Ok(());
                            }
                            Some(event) = event_receiver.recv() => {
                                outgoing.send(Message::Event { id, event }).await.map_err(|_| ConnectionError::OwnerGone)?;
                            }
                        }
                    }
                });
            }
            Some(message) = replies.recv() => {
                let shutdown = matches!(message, Message::Response { response: Response::ShuttingDown, .. });
                writer.send(&message).await?;
                if shutdown { return Ok(ConnectionEnd::ShutdownAccepted); }
            }
            Some(result) = tasks.join_next() => {
                match result {
                    Ok(Ok(())) => {},
                    Ok(Err(error)) => return Err(error),
                    Err(error) => tracing::warn!(%error, "a connection task failed"),
                }
            }
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
    use crate::ipc::{Event, RequestId};

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
    #[tokio::test]
    async fn connection_delivers_out_of_order_replies_and_watch_events() {
        let (near, far) = duplex(4096);
        let (sender, mut requests) = mpsc::channel(8);
        let server = tokio::spawn(serve_connection(Connection::new(far), sender));
        let mut client = Connection::new(near);
        client
            .send(&Message::Hello {
                version: VERSION.into(),
            })
            .await
            .unwrap();
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(Message::Hello { .. })
        ));
        for id in [1, 2] {
            client
                .send(&Message::Request {
                    id: RequestId(id),
                    request: Request::Status,
                })
                .await
                .unwrap();
        }
        let first = requests.recv().await.unwrap();
        let second = tokio::time::timeout(Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        second
            .reply
            .send(Response::Tools(serde_json::json!({"value":2})))
            .unwrap();
        let got = client.recv().await.unwrap();
        assert!(got.is_some(), "server ended: {:?}", server.is_finished());
        assert_eq!(
            got,
            Some(Message::Response {
                id: RequestId(2),
                response: Response::Tools(serde_json::json!({"value":2}))
            })
        );
        first
            .reply
            .send(Response::Tools(serde_json::json!({"value":1})))
            .unwrap();
        assert_eq!(
            client.recv().await.unwrap(),
            Some(Message::Response {
                id: RequestId(1),
                response: Response::Tools(serde_json::json!({"value":1}))
            })
        );
        client
            .send(&Message::Request {
                id: RequestId(3),
                request: Request::WatchSession {
                    session: "app_12-00-00".parse().unwrap(),
                },
            })
            .await
            .unwrap();
        let watch = requests.recv().await.unwrap();
        let event = Event::Progress {
            message: "working".into(),
        };
        watch.events.unwrap().send(event.clone()).await.unwrap();
        assert_eq!(
            client.recv().await.unwrap(),
            Some(Message::Event {
                id: RequestId(3),
                event
            })
        );
        watch.reply.send(Response::WatchEnded).unwrap();
        assert_eq!(
            client.recv().await.unwrap(),
            Some(Message::Response {
                id: RequestId(3),
                response: Response::WatchEnded
            })
        );
        drop(client);
        server.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn dropped_session_reply_does_not_close_connection() {
        let (near, far) = duplex(4096);
        let (sender, mut requests) = mpsc::channel(8);
        let server = tokio::spawn(serve_connection(Connection::new(far), sender));
        let mut client = Connection::new(near);
        client
            .send(&Message::Hello {
                version: VERSION.into(),
            })
            .await
            .unwrap();
        client.recv().await.unwrap();
        client
            .send(&Message::Request {
                id: RequestId(1),
                request: Request::Status,
            })
            .await
            .unwrap();
        drop(requests.recv().await.unwrap());
        assert!(
            matches!(client.recv().await.unwrap(), Some(Message::Response { response: Response::Refused { message }, .. }) if message.contains("ended before it answered"))
        );
        client
            .send(&Message::Request {
                id: RequestId(2),
                request: Request::Status,
            })
            .await
            .unwrap();
        requests
            .recv()
            .await
            .unwrap()
            .reply
            .send(Response::WatchEnded)
            .unwrap();
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(Message::Response {
                id: RequestId(2),
                ..
            })
        ));
        drop(client);
        server.await.unwrap().unwrap();
    }
}
