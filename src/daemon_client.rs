//! How `keek mcp` reaches the daemon: connect, start one if none is running, and
//! replace an older one when ADR 0015 allows it.

use std::collections::HashMap;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use semver::Version;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::VERSION;
use crate::ipc::{Connection, DaemonStatus, IpcError, Message, Request, RequestId, Response};
use crate::ipc::{ConnectionReader, ConnectionWriter, Event};
use crate::paths::Paths;
use crate::session_folder::SessionName;

const RETRY_INTERVAL: Duration = Duration::from_millis(25);

pub trait DaemonStarter {
    /// Called on every retry while no daemon answers, so one start can see many calls.
    fn start(&mut self) -> io::Result<()>;
}

/// The daemon inherits this process's environment, which is how it gets the host's
/// Wayland and Hyprland variables.
#[derive(Debug)]
pub struct ProcessStarter {
    executable: PathBuf,
    child: Option<tokio::process::Child>,
}

impl ProcessStarter {
    #[must_use]
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            child: None,
        }
    }
}

impl DaemonStarter for ProcessStarter {
    fn start(&mut self) -> io::Result<()> {
        if let Some(child) = &mut self.child {
            match child.try_wait()? {
                // Still starting, so it only needs more time.
                None => return Ok(()),
                // It lost the lock to a daemon that is exiting, so start another.
                Some(status) if status.success() => {}
                Some(status) => {
                    return Err(io::Error::other(format!(
                        "keek daemon exited with {status}"
                    )));
                }
            }
        }
        let mut command = std::process::Command::new(&self.executable);
        command
            .arg("daemon")
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        self.child = Some(tokio::process::Command::from(command).spawn()?);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error(transparent)]
    Ipc(IpcError),
    #[error("The keek daemon closed the connection. Restart the coding agent's keek MCP server.")]
    DaemonGone,
    #[error("the keek daemon sent an unexpected message: {0}")]
    Unexpected(String),
}

impl From<IpcError> for RequestError {
    fn from(error: IpcError) -> Self {
        match error {
            IpcError::Io(io)
                if matches!(
                    io.kind(),
                    io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                ) =>
            {
                Self::DaemonGone
            }
            other => Self::Ipc(other),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error(
        "keek mcp {ours} can't use keek daemon {theirs}, and the daemon can't be replaced while an Agent Session is open ({open_sessions} open). Close the open sessions, then start keek mcp again."
    )]
    VersionMismatch {
        ours: String,
        theirs: String,
        open_sessions: usize,
    },
    #[error(
        "keek mcp {ours} is older than the running keek daemon {theirs}. Point the coding agent at the newer keek binary, then start keek mcp again."
    )]
    DaemonIsNewer { ours: String, theirs: String },
    #[error(
        "The keek daemon did not answer on {socket} within {timeout:?}. Check its log at {log}, then start keek mcp again.",
        socket = .socket.display(),
        log = .log.display()
    )]
    Timeout {
        socket: PathBuf,
        log: PathBuf,
        timeout: Duration,
    },
    #[error("Could not start the keek daemon. Check its log at {log}, then start keek mcp again.", log = .log.display())]
    Start {
        log: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not connect to the keek daemon socket")]
    Connect(#[source] io::Error),
    #[error(transparent)]
    Request(#[from] RequestError),
}

pub async fn connect_or_start(
    paths: &Paths,
    starter: &mut impl DaemonStarter,
    timeout: Duration,
) -> Result<DaemonClient, ConnectError> {
    tokio::time::timeout(timeout, retry_until_connected(paths, starter))
        .await
        .map_err(|_| ConnectError::Timeout {
            socket: paths.socket(),
            log: paths.daemon_log(),
            timeout,
        })?
}

async fn retry_until_connected(
    paths: &Paths,
    starter: &mut impl DaemonStarter,
) -> Result<DaemonClient, ConnectError> {
    let socket = paths.socket();
    loop {
        match UnixStream::connect(&socket).await {
            Ok(stream) => {
                if let Some(connection) = negotiate(Connection::new(stream)).await? {
                    return Ok(DaemonClient::new(connection));
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                starter.start().map_err(|source| ConnectError::Start {
                    log: paths.daemon_log(),
                    source,
                })?;
            }
            Err(error) => return Err(ConnectError::Connect(error)),
        }
        tokio::time::sleep(RETRY_INTERVAL).await;
    }
}

/// Returns the connection when the daemon matches this version, and `None` when it
/// has gone or agreed to exit, so the caller tries again. An idling daemon can close
/// a connection it has just accepted, which is why a hang-up isn't an error here.
async fn negotiate<S>(mut connection: Connection<S>) -> Result<Option<Connection<S>>, ConnectError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let theirs = match hello(&mut connection).await {
        Ok(theirs) => theirs,
        Err(RequestError::DaemonGone) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if theirs == VERSION {
        return Ok(Some(connection));
    }
    // ADR 0015 replaces only an older daemon. Without this, two installed builds would
    // keep evicting each other's daemon.
    if is_newer_than_this_build(&theirs) {
        return Err(ConnectError::DaemonIsNewer {
            ours: VERSION.to_owned(),
            theirs,
        });
    }

    // The daemon decides, so whether a session is open is never read stale here.
    match exchange(&mut connection, RequestId(0), Request::Shutdown).await {
        Ok(Response::ShuttingDown) | Err(RequestError::DaemonGone) => Ok(None),
        Ok(Response::ShutdownRefused { open_sessions }) => Err(ConnectError::VersionMismatch {
            ours: VERSION.to_owned(),
            theirs,
            open_sessions,
        }),
        Ok(
            other @ (Response::Status(_)
            | Response::Tools(_)
            | Response::Opened { .. }
            | Response::ToolResult(_)
            | Response::CheckAdded { .. }
            | Response::CheckMarked { .. }
            | Response::Closed(_)
            | Response::Refused { .. }
            | Response::WatchEnded),
        ) => Err(RequestError::Unexpected(format!("{other:?}")).into()),
        Err(error) => Err(error.into()),
    }
}

fn is_newer_than_this_build(theirs: &str) -> bool {
    match (Version::parse(theirs), Version::parse(VERSION)) {
        (Ok(theirs), Ok(ours)) => theirs > ours,
        (Err(_), _) | (_, Err(_)) => false,
    }
}

async fn hello<S>(connection: &mut Connection<S>) -> Result<String, RequestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    connection
        .send(&Message::Hello {
            version: VERSION.to_owned(),
        })
        .await?;
    match connection.recv().await? {
        Some(Message::Hello { version }) => Ok(version),
        Some(other) => Err(RequestError::Unexpected(format!("{other:?}"))),
        None => Err(RequestError::DaemonGone),
    }
}

async fn exchange<S>(
    connection: &mut Connection<S>,
    id: RequestId,
    request: Request,
) -> Result<Response, RequestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    connection.send(&Message::Request { id, request }).await?;
    loop {
        match connection.recv().await? {
            Some(Message::Response {
                id: answered,
                response,
            }) if answered == id => return Ok(response),
            // Nothing in `keek mcp` reads events yet.
            Some(Message::Event { .. }) => {}
            Some(
                other
                @ (Message::Hello { .. } | Message::Request { .. } | Message::Response { .. }),
            ) => {
                return Err(RequestError::Unexpected(format!("{other:?}")));
            }
            None => return Err(RequestError::DaemonGone),
        }
    }
}

#[derive(Default)]
struct PendingState {
    closed: bool,
    replies: HashMap<RequestId, PendingReply>,
}
type Pending = Arc<Mutex<PendingState>>;
type Writer = Arc<Mutex<ConnectionWriter<Box<dyn AsyncWrite + Unpin + Send>>>>;

#[derive(Debug)]
struct PendingReply {
    response: oneshot::Sender<Result<Response, RequestError>>,
    events: Option<mpsc::Sender<Event>>,
}

/// Dropping it closes the connection and its reader task.
pub struct DaemonClient {
    writer: Writer,
    pending: Pending,
    next_id: AtomicU64,
    reader: JoinHandle<()>,
}

impl std::fmt::Debug for DaemonClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonClient")
            .finish_non_exhaustive()
    }
}

impl Drop for DaemonClient {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl DaemonClient {
    fn new<S>(connection: Connection<S>) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (reader, writer) = connection.split();
        let pending = Arc::new(Mutex::new(PendingState::default()));
        let writer = Arc::new(Mutex::new(writer.boxed()));
        let task = tokio::spawn(read_responses(
            reader,
            Arc::clone(&pending),
            Arc::clone(&writer),
        ));
        Self {
            writer,
            pending,
            next_id: AtomicU64::new(1),
            reader: task,
        }
    }

    async fn start_request(
        &self,
        request: Request,
        events: Option<mpsc::Sender<Event>>,
    ) -> Result<oneshot::Receiver<Result<Response, RequestError>>, RequestError> {
        let id = RequestId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (reply, answer) = oneshot::channel();
        let mut pending = self.pending.lock().await;
        if pending.closed {
            return Err(RequestError::DaemonGone);
        }
        pending.replies.insert(
            id,
            PendingReply {
                response: reply,
                events,
            },
        );
        drop(pending);
        if let Err(error) = self
            .writer
            .lock()
            .await
            .send(&Message::Request { id, request })
            .await
        {
            self.pending.lock().await.replies.remove(&id);
            return Err(error.into());
        }
        Ok(answer)
    }

    pub async fn request(&self, request: Request) -> Result<Response, RequestError> {
        self.start_request(request, None)
            .await?
            .await
            .map_err(|_| RequestError::DaemonGone)?
    }

    pub async fn watch_session(
        &self,
        session: SessionName,
    ) -> Result<mpsc::Receiver<Event>, RequestError> {
        let (sender, receiver) = mpsc::channel(16);
        // The response removes the pending sender and closes the event receiver.
        let _answer = self
            .start_request(Request::WatchSession { session }, Some(sender))
            .await?;
        Ok(receiver)
    }

    pub async fn status(&self) -> Result<DaemonStatus, RequestError> {
        match self.request(Request::Status).await? {
            Response::Status(status) => Ok(status),
            other @ (Response::ShuttingDown
            | Response::ShutdownRefused { .. }
            | Response::Tools(_)
            | Response::Opened { .. }
            | Response::ToolResult(_)
            | Response::CheckAdded { .. }
            | Response::CheckMarked { .. }
            | Response::Closed(_)
            | Response::Refused { .. }
            | Response::WatchEnded) => Err(RequestError::Unexpected(format!("{other:?}"))),
        }
    }
}

async fn read_responses<R: AsyncRead + Unpin>(
    mut reader: ConnectionReader<R>,
    pending: Pending,
    writer: Writer,
) {
    loop {
        match reader.recv().await {
            Ok(Some(Message::Response { id, response })) => {
                if let Some(reply) = pending.lock().await.replies.remove(&id) {
                    let _ = reply.response.send(Ok(response));
                }
            }
            Ok(Some(Message::Event { id, event })) => {
                if let Some(sender) = pending
                    .lock()
                    .await
                    .replies
                    .get(&id)
                    .and_then(|reply| reply.events.as_ref())
                {
                    let _ = sender.try_send(event);
                }
            }
            Ok(Some(Message::Hello { .. } | Message::Request { .. })) | Ok(None) | Err(_) => break,
        }
    }
    let mut pending = pending.lock().await;
    pending.closed = true;
    for (_, reply) in pending.replies.drain() {
        let _ = reply.response.send(Err(RequestError::DaemonGone));
    }
    drop(pending);
    let _ = writer.lock().await.shutdown().await;
}

#[cfg(test)]
#[path = "daemon_client_tests.rs"]
mod tests;
