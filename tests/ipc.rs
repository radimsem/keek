use anyhow::Result;
use keek::ipc::{
    Connection, DaemonStatus, Event, Message, Request, RequestId, Response, SessionState,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex};

fn every_message() -> Result<Vec<Message>> {
    let id = RequestId(7);
    let session: keek::session_folder::SessionName = "app_12-00-00".parse()?;
    Ok(vec![
        Message::Hello {
            version: "0.1.0".to_owned(),
        },
        Message::Request {
            id,
            request: Request::Status,
        },
        Message::Request {
            id,
            request: Request::Shutdown,
        },
        Message::Request {
            id,
            request: Request::ListTools,
        },
        Message::Request {
            id,
            request: Request::OpenSession {
                project: "app".into(),
                worktree: "/tmp/app".into(),
                goal: "goal".into(),
                checks: vec!["check".into()],
            },
        },
        Message::Request {
            id,
            request: Request::CallTool {
                session: session.clone(),
                tool: "click".into(),
                arguments: serde_json::json!({}),
            },
        },
        Message::Request {
            id,
            request: Request::CloseSession {
                session: session.clone(),
                summary: "done".into(),
            },
        },
        Message::Request {
            id,
            request: Request::WatchSession {
                session: session.clone(),
            },
        },
        Message::Response {
            id,
            response: Response::Tools(serde_json::json!({"tools":[]})),
        },
        Message::Response {
            id,
            response: Response::Opened {
                session: session.clone(),
            },
        },
        Message::Response {
            id,
            response: Response::ToolResult(serde_json::json!({})),
        },
        Message::Response {
            id,
            response: Response::Closed(serde_json::json!({})),
        },
        Message::Response {
            id,
            response: Response::Refused {
                message: "no".into(),
            },
        },
        Message::Response {
            id,
            response: Response::WatchEnded,
        },
        Message::Event {
            id,
            event: Event::StateChanged {
                state: SessionState::Open,
            },
        },
        Message::Event {
            id,
            event: Event::LastAction {
                tool: "click".into(),
                time_ms: keek::timeline::Millis(1),
            },
        },
        Message::Response {
            id,
            response: Response::Status(DaemonStatus {
                version: "0.1.0".to_owned(),
                pid: 42,
                open_sessions: 1,
            }),
        },
        Message::Response {
            id,
            response: Response::ShuttingDown,
        },
        Message::Response {
            id,
            response: Response::ShutdownRefused { open_sessions: 2 },
        },
        Message::Event {
            id,
            event: Event::Progress {
                message: "Waiting for the developer.".to_owned(),
            },
        },
    ])
}

#[tokio::test]
async fn every_message_crosses_a_duplex_unchanged() -> Result<()> {
    let (near, far) = duplex(4096);
    let mut sender = Connection::new(near);
    let mut receiver = Connection::new(far);

    for message in every_message()? {
        sender.send(&message).await?;
        assert_eq!(receiver.recv().await?, Some(message));
    }
    Ok(())
}

#[tokio::test]
async fn a_closed_peer_reads_as_the_end_of_the_stream() -> Result<()> {
    let (near, far) = duplex(64);
    drop(near);

    assert_eq!(Connection::new(far).recv().await?, None);
    Ok(())
}

#[tokio::test]
async fn each_message_is_one_line_of_json() -> Result<()> {
    let (near, far) = duplex(4096);
    let mut sender = Connection::new(near);
    let mut reader = BufReader::new(far);

    sender
        .send(&Message::Hello {
            version: "0.1.0".to_owned(),
        })
        .await?;
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    assert_eq!(line, "{\"type\":\"hello\",\"version\":\"0.1.0\"}\n");
    Ok(())
}

/// A new `keek mcp` replaces an older daemon with these, so they can't change.
#[test]
fn the_messages_that_replace_an_old_daemon_keep_their_exact_shape() -> Result<()> {
    let id = RequestId(1);
    let cases = [
        (
            Message::Hello {
                version: "0.1.0".to_owned(),
            },
            r#"{"type":"hello","version":"0.1.0"}"#,
        ),
        (
            Message::Request {
                id,
                request: Request::Shutdown,
            },
            r#"{"type":"request","id":1,"request":{"kind":"shutdown"}}"#,
        ),
        (
            Message::Response {
                id,
                response: Response::ShuttingDown,
            },
            r#"{"type":"response","id":1,"response":{"kind":"shutting_down"}}"#,
        ),
        (
            Message::Response {
                id,
                response: Response::ShutdownRefused { open_sessions: 2 },
            },
            r#"{"type":"response","id":1,"response":{"kind":"shutdown_refused","open_sessions":2}}"#,
        ),
    ];

    for (message, expected) in cases {
        assert_eq!(serde_json::to_string(&message)?, expected);
    }
    Ok(())
}

#[tokio::test]
async fn a_line_that_is_not_a_message_is_an_error() -> Result<()> {
    let (mut near, far) = duplex(64);
    near.write_all(b"not json\n").await?;

    assert!(Connection::new(far).recv().await.is_err());
    Ok(())
}
