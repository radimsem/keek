//! The per-user daemon's lifecycle: one instance through a lock file, one owner
//! loop over its state, and exit once nothing has needed it for the idle limit.

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::clock::Clock;

pub const IDLE_LIMIT_ENV: &str = "KEEK_DAEMON_IDLE_MS";
const DEFAULT_IDLE_LIMIT: Duration = Duration::from_secs(60);
pub const SESSION_IDLE_LIMIT_ENV: &str = "KEEK_SESSION_IDLE_MS";

pub fn session_idle_limit(value: Option<&str>) -> Result<Duration, DaemonError> {
    value.map_or(Ok(Duration::from_secs(300)), |value| {
        value
            .parse::<u64>()
            .ok()
            .filter(|millis| *millis > 0)
            .map(Duration::from_millis)
            .ok_or_else(|| DaemonError::InvalidSessionIdleLimit(value.to_owned()))
    })
}
const REQUEST_QUEUE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Exit {
    Idle,
    ShutDown,
    Terminated,
    /// Another daemon holds the lock, so this one never served.
    AlreadyRunning,
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error("{IDLE_LIMIT_ENV} must be a whole number of milliseconds, not {0:?}")]
    InvalidIdleLimit(String),
    #[error("KEEK_SESSION_IDLE_MS must be a positive whole number of milliseconds, not {0:?}")]
    InvalidSessionIdleLimit(String),
    #[error("could not create the runtime folder")]
    RuntimeDir(#[source] io::Error),
    #[error("could not take the daemon lock")]
    Lock(#[source] io::Error),
    #[error("could not bind the daemon socket")]
    Bind(#[source] io::Error),
    #[error("could not listen for SIGTERM")]
    Signal(#[source] io::Error),
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

#[path = "daemon_connection.rs"]
mod connection;
use connection::{ConnectionEnd, PendingRequest, serve_client};
#[path = "daemon_start.rs"]
mod start;
pub use start::{run, run_with_viewer};

fn remove_socket(socket: &Path) -> Result<(), DaemonError> {
    match std::fs::remove_file(socket) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            Err(DaemonError::RemoveSocket(error))
        }
        Ok(()) | Err(_) => Ok(()),
    }
}

pub(crate) async fn serve(
    listener: UnixListener,
    mut idle: IdleWatch<impl Clock>,
    mut registry: crate::registry::Registry,
    terminated: impl std::future::Future<Output = ()>,
) -> Result<Exit, DaemonError> {
    registry.remove_host_rule().await;
    tokio::pin!(terminated);
    let (request_sender, mut requests) = mpsc::channel::<PendingRequest>(REQUEST_QUEUE);
    let mut connections: JoinSet<ConnectionEnd> = JoinSet::new();
    let mut commands = registry
        .dependencies
        .viewer
        .take_commands()
        .unwrap_or_else(|| mpsc::channel(1).1);

    loop {
        idle.update(registry.keeps_daemon_alive() || !connections.is_empty());
        let remaining = idle.remaining();

        tokio::select! {
            () = &mut terminated => {
                registry.remove_host_rule().await;
                return Ok(Exit::Terminated);
            }
            Some(command) = commands.recv() => registry.control(command).await,
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
            Some(ended) = registry.recovery.join_next() => { if let Err(error) = ended { tracing::error!(%error, "session recovery task failed"); } }
            Some(ended) = registry.background.join_next() => { if let Err(error) = ended { tracing::error!(%error, "daemon background task failed"); } }
            Some(ended) = connections.join_next() => {
                match ended {
                    Ok(ConnectionEnd::ShutdownAccepted) => return Ok(Exit::ShutDown),
                    Ok(ConnectionEnd::ClientLeft) => {},
                    Err(error) => tracing::error!(%error, "a client connection task failed"),
                }
            }
            Some(pending) = requests.recv() => {
                registry.handle_connection(Some(pending.opener), pending.request, pending.reply, pending.events).await;
            }
            () = tokio::time::sleep(remaining.unwrap_or_default()), if remaining.is_some() => {
                return Ok(Exit::Idle);
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct IdleWatch<C> {
    clock: C,
    limit: Duration,
    idle_since: Option<Instant>,
}

impl<C: Clock> IdleWatch<C> {
    pub(crate) fn new(clock: C, limit: Duration) -> Self {
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

    use super::*;

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

    #[test]
    fn session_idle_limit_defaults_to_five_minutes_and_accepts_milliseconds() {
        assert_eq!(session_idle_limit(None).unwrap(), Duration::from_secs(300));
        assert_eq!(
            session_idle_limit(Some("3000")).unwrap(),
            Duration::from_secs(3)
        );
        assert!(session_idle_limit(Some("soon")).is_err());
        assert!(session_idle_limit(Some("0")).is_err());
    }
}
