//! How `keek mcp` reaches the daemon: connect, start one if none is running, and
//! replace an older one when ADR 0015 allows it.

use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use semver::Version;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use crate::VERSION;
use crate::ipc::{Connection, DaemonStatus, IpcError, Message, Request, RequestId, Response};
use crate::paths::Paths;

const RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// Starts a daemon when none answers on the socket.
pub trait DaemonStarter {
    /// # Errors
    ///
    /// Fails when the daemon's process can't be started, or the last one failed.
    fn start(&mut self) -> io::Result<()>;
}

/// Starts `keek daemon` detached from `keek mcp`. It inherits this process's
/// environment, which is how it gets the host's Wayland and Hyprland variables.
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

/// Connects to the daemon on `paths`, starting or replacing it as needed, for up to
/// `timeout`.
///
/// # Errors
///
/// Fails when an older daemon has an open Agent Session, the daemon is newer than this
/// build, the daemon can't be started, or none answers in time.
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
        Ok(other @ Response::Status(_)) => {
            Err(RequestError::Unexpected(format!("{other:?}")).into())
        }
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

/// A live connection to the daemon. Dropping it tells the daemon this client left.
#[derive(Debug)]
pub struct DaemonClient {
    state: Mutex<ClientState>,
}

#[derive(Debug)]
struct ClientState {
    connection: Connection<UnixStream>,
    next_id: u64,
}

impl DaemonClient {
    fn new(connection: Connection<UnixStream>) -> Self {
        Self {
            state: Mutex::new(ClientState {
                connection,
                next_id: 1,
            }),
        }
    }

    /// Asks the daemon for its version, pid and open session count.
    ///
    /// # Errors
    ///
    /// Fails when the daemon has gone or answers with something else.
    pub async fn status(&self) -> Result<DaemonStatus, RequestError> {
        let mut state = self.state.lock().await;
        let id = RequestId(state.next_id);
        state.next_id += 1;
        match exchange(&mut state.connection, id, Request::Status).await? {
            Response::Status(status) => Ok(status),
            other @ (Response::ShuttingDown | Response::ShutdownRefused { .. }) => {
                Err(RequestError::Unexpected(format!("{other:?}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::duplex;

    use super::*;
    use crate::ipc::Event;

    struct NoDaemon;

    impl DaemonStarter for NoDaemon {
        fn start(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BrokenStarter;

    impl DaemonStarter for BrokenStarter {
        fn start(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
    }

    fn status() -> Response {
        Response::Status(DaemonStatus {
            version: VERSION.to_owned(),
            pid: 1,
            open_sessions: 0,
        })
    }

    #[tokio::test]
    async fn exchange_refuses_a_response_to_another_request() {
        let (near, far) = duplex(4096);
        let mut client = Connection::new(near);
        let mut daemon = Connection::new(far);

        daemon
            .send(&Message::Response {
                id: RequestId(4),
                response: status(),
            })
            .await
            .unwrap();

        assert!(matches!(
            exchange(&mut client, RequestId(5), Request::Status).await,
            Err(RequestError::Unexpected(_))
        ));
    }

    #[test]
    fn only_a_higher_version_counts_as_newer() {
        assert!(is_newer_than_this_build("99.0.0"));
        assert!(!is_newer_than_this_build("0.0.1"));
        assert!(!is_newer_than_this_build(VERSION));
        assert!(!is_newer_than_this_build("not a version"));
    }

    #[tokio::test]
    async fn exchange_skips_events_before_its_response() {
        let (near, far) = duplex(4096);
        let mut client = Connection::new(near);
        let mut daemon = Connection::new(far);
        let id = RequestId(5);

        daemon
            .send(&Message::Event {
                id,
                event: Event::Progress {
                    message: "Working.".to_owned(),
                },
            })
            .await
            .unwrap();
        daemon
            .send(&Message::Response {
                id,
                response: status(),
            })
            .await
            .unwrap();

        assert_eq!(
            exchange(&mut client, id, Request::Status).await.unwrap(),
            status()
        );
    }

    #[tokio::test]
    async fn exchange_reports_a_daemon_that_hangs_up() {
        let (near, far) = duplex(4096);
        let mut client = Connection::new(near);
        let mut daemon = Connection::new(far);

        let (result, request) = tokio::join!(
            exchange(&mut client, RequestId(1), Request::Status),
            async {
                let request = daemon.recv().await;
                drop(daemon);
                request
            }
        );

        assert!(request.unwrap().is_some());
        assert!(matches!(result, Err(RequestError::DaemonGone)));
    }

    #[tokio::test]
    async fn a_daemon_that_never_answers_times_out_and_names_its_log() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under_root(dir.path());

        let error = connect_or_start(&paths, &mut NoDaemon, Duration::from_millis(100))
            .await
            .unwrap_err();

        let message = error.to_string();
        assert!(matches!(error, ConnectError::Timeout { .. }));
        assert!(message.contains("daemon.log"), "message was: {message}");
    }

    #[tokio::test]
    async fn a_daemon_that_cannot_start_is_reported_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under_root(dir.path());

        let error = connect_or_start(&paths, &mut BrokenStarter, Duration::from_secs(5))
            .await
            .unwrap_err();

        assert!(matches!(error, ConnectError::Start { .. }));
    }
}
