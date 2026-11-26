use tokio::io::duplex;

use super::*;
use crate::ipc::{Event, LastAction, SessionSnapshot, SessionState};

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
        build: None,
        pid: 1,
        open_sessions: 0,
        decision_model: None,
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

#[tokio::test]
async fn one_in_flight_request_does_not_block_the_next_request() {
    let (near, far) = duplex(4096);
    let client = DaemonClient::new(Connection::new(near));
    let daemon = tokio::spawn(async move {
        let mut connection = Connection::new(far);
        let Some(Message::Request { id: first, .. }) = connection.recv().await.unwrap() else {
            panic!("the client sent no first request");
        };
        let second = tokio::time::timeout(Duration::from_millis(100), connection.recv()).await;
        assert!(
            second.is_ok(),
            "the second request waited for the first reply"
        );
        let Some(Message::Request { id: second, .. }) = second.unwrap().unwrap() else {
            panic!("the client sent no second request");
        };
        connection
            .send(&Message::Response {
                id: second,
                response: status(),
            })
            .await
            .unwrap();
        connection
            .send(&Message::Response {
                id: first,
                response: status(),
            })
            .await
            .unwrap();
    });
    let (first, second) = tokio::join!(client.status(), client.status());
    daemon.await.unwrap();
    assert!(first.is_ok());
    assert!(second.is_ok());
}

#[tokio::test]
async fn watch_events_reach_only_the_matching_request() {
    let (near, far) = duplex(4096);
    let client = DaemonClient::new(Connection::new(near));
    let mut daemon = Connection::new(far);
    let mut first = client
        .watch_session("one_12-00-00".parse().unwrap())
        .await
        .unwrap();
    let mut second = client
        .watch_session("two_12-00-00".parse().unwrap())
        .await
        .unwrap();
    let Some(Message::Request { id: first_id, .. }) = daemon.recv().await.unwrap() else {
        panic!("no first watch");
    };
    let Some(Message::Request { id: second_id, .. }) = daemon.recv().await.unwrap() else {
        panic!("no second watch");
    };
    let event = Event::Snapshot(SessionSnapshot {
        control: crate::ipc::ControlState::Verifier,
        state: SessionState::Open,
        checks: vec![],
        reminded: false,
        last_action: Some(LastAction {
            tool: "click".to_owned(),
            time_ms: crate::timeline::Millis(24),
        }),
    });
    daemon
        .send(&Message::Event {
            id: second_id,
            event: event.clone(),
        })
        .await
        .unwrap();
    assert_eq!(second.recv().await, Some(event));
    assert!(first.try_recv().is_err());
    daemon
        .send(&Message::Response {
            id: second_id,
            response: Response::WatchEnded,
        })
        .await
        .unwrap();
    assert_eq!(second.recv().await, None);
    daemon
        .send(&Message::Response {
            id: first_id,
            response: Response::WatchEnded,
        })
        .await
        .unwrap();
    assert_eq!(first.recv().await, None);
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
    assert!(
        matches!(error, ConnectError::Timeout { .. }),
        "actual error: {error:?}"
    );
    assert!(message.contains("daemon.log"), "message was: {message}");
}

#[tokio::test]
async fn a_daemon_that_cannot_start_is_reported_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under_root(dir.path());

    let error = connect_or_start(&paths, &mut BrokenStarter, Duration::from_secs(5))
        .await
        .unwrap_err();

    assert!(
        matches!(error, ConnectError::Start { .. }),
        "actual error: {error:?}"
    );
}

#[tokio::test]
async fn request_after_invalid_message_returns_daemon_gone() {
    let (near, far) = duplex(4096);
    let client = DaemonClient::new(Connection::new(near));
    let mut daemon = Connection::new(far);
    daemon
        .send(&Message::Request {
            id: RequestId(99),
            request: Request::Status,
        })
        .await
        .unwrap();
    while !client.reader.is_finished() {
        tokio::task::yield_now().await;
    }
    let result =
        tokio::time::timeout(Duration::from_millis(100), client.request(Request::Status)).await;
    assert!(
        matches!(result, Ok(Err(RequestError::DaemonGone))),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_cancelled_request_tells_the_daemon_and_drops_its_pending_reply() {
    let (near, far) = duplex(4096);
    let client = DaemonClient::new(Connection::new(near));
    let mut daemon = Connection::new(far);
    let (cancel, cancelled) = oneshot::channel::<()>();
    let request = client.request_until(Request::Status, None, async {
        let _ = cancelled.await;
    });
    let daemon_side = async {
        let Some(Message::Request { id, .. }) = daemon.recv().await.unwrap() else {
            panic!("the client sent no request");
        };
        cancel.send(()).unwrap();
        (id, daemon.recv().await.unwrap())
    };

    let (result, (id, next)) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(request, daemon_side)
    })
    .await
    .expect("the cancel never reached the daemon");

    assert!(matches!(result, Err(RequestError::Cancelled)), "{result:?}");
    assert_eq!(next, Some(Message::Cancel { id }));
    assert!(client.pending.lock().await.replies.is_empty());
}

#[tokio::test]
async fn a_started_daemon_writes_its_stderr_to_the_daemon_log() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("state/daemon.log");
    // `sh daemon` finds no script named daemon in `/` and says so on stderr.
    let mut starter = ProcessStarter::new("/bin/sh".into(), log.clone());

    starter.start().unwrap();
    starter.child.as_mut().unwrap().wait().await.unwrap();

    assert!(std::fs::read_to_string(&log).unwrap().contains("daemon"));
}

fn executable_build_nanos() -> u64 {
    std::fs::metadata("/proc/self/exe")
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .try_into()
        .unwrap()
}

fn build_timestamp(nanos: u64) -> String {
    chrono::DateTime::from_timestamp(
        (nanos / 1_000_000_000) as i64,
        (nanos % 1_000_000_000) as u32,
    )
    .unwrap()
    .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

async fn build_handshake(
    build: Option<u64>,
    response: Response,
) -> (
    Result<Option<Connection<tokio::io::DuplexStream>>, ConnectError>,
    Connection<tokio::io::DuplexStream>,
) {
    let (near, far) = duplex(4096);
    let mut daemon = Connection::new(far);
    let mut hello = serde_json::json!({"type":"hello", "version":VERSION});
    if let Some(build) = build {
        hello["build"] = build.into();
    }
    daemon
        .send(&serde_json::from_value(hello).unwrap())
        .await
        .unwrap();
    daemon
        .send(&Message::Response {
            id: RequestId(0),
            response,
        })
        .await
        .unwrap();
    let result = negotiate(Connection::new(near)).await;
    assert!(matches!(
        daemon.recv().await.unwrap(),
        Some(Message::Hello { .. })
    ));
    (result, daemon)
}

#[tokio::test]
async fn the_same_version_with_an_older_daemon_build_requests_shutdown() {
    let (result, mut daemon) = build_handshake(
        Some(executable_build_nanos() - 1_000_000_000),
        Response::ShuttingDown,
    )
    .await;
    assert!(matches!(result, Ok(None)), "{result:?}");
    assert!(matches!(
        daemon.recv().await.unwrap(),
        Some(Message::Request {
            request: Request::Shutdown,
            ..
        })
    ));
}

#[tokio::test]
async fn a_daemon_without_a_build_requests_shutdown() {
    let (result, mut daemon) = build_handshake(None, Response::ShuttingDown).await;
    assert!(matches!(result, Ok(None)), "{result:?}");
    assert!(matches!(
        daemon.recv().await.unwrap(),
        Some(Message::Request {
            request: Request::Shutdown,
            ..
        })
    ));
}

#[tokio::test]
async fn a_newer_daemon_build_is_refused_without_shutdown() {
    let ours = executable_build_nanos();
    let theirs = ours + 1_000_000_000;
    let (result, mut daemon) = build_handshake(Some(theirs), Response::ShuttingDown).await;
    assert!(
        matches!(&result, Err(ConnectError::DaemonIsNewer { .. })),
        "{result:?}"
    );
    let message = result.unwrap_err().to_string();
    assert!(message.contains(&build_timestamp(ours)), "{message}");
    assert!(message.contains(&build_timestamp(theirs)), "{message}");
    assert_eq!(daemon.recv().await.unwrap(), None);
}

#[tokio::test]
async fn an_equal_daemon_build_connects() {
    let (result, mut daemon) =
        build_handshake(Some(executable_build_nanos()), Response::ShuttingDown).await;
    assert!(matches!(&result, Ok(Some(_))), "{result:?}");
    drop(result);
    assert_eq!(daemon.recv().await.unwrap(), None);
}

#[tokio::test]
async fn an_older_build_with_a_session_open_is_refused_naming_both_builds() {
    let ours = executable_build_nanos();
    let theirs = ours - 1_000_000_000;
    let (result, mut daemon) =
        build_handshake(Some(theirs), Response::ShutdownRefused { open_sessions: 1 }).await;
    assert!(
        matches!(
            &result,
            Err(ConnectError::VersionMismatch {
                open_sessions: 1,
                ..
            })
        ),
        "{result:?}"
    );
    let message = result.unwrap_err().to_string();
    assert!(message.contains(&build_timestamp(ours)), "{message}");
    assert!(message.contains(&build_timestamp(theirs)), "{message}");
    assert!(matches!(
        daemon.recv().await.unwrap(),
        Some(Message::Request {
            request: Request::Shutdown,
            ..
        })
    ));
}
