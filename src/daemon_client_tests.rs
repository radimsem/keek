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
    let event = Event::LastAction {
        tool: "click".to_owned(),
        time_ms: crate::timeline::Millis(24),
    };
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
