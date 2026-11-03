use super::*;
#[tokio::test(start_paused = true)]
async fn call_tool_forwards_progress_token_to_the_client() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (daemon_stream, fake_stream) = tokio::io::duplex(4096);
    let daemon = DaemonClient::new(crate::ipc::Connection::new(daemon_stream));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let fake = tokio::spawn(async move {
        let mut connection = crate::ipc::Connection::new(fake_stream);
        let message = connection.recv().await.unwrap().unwrap();
        assert!(matches!(
            message,
            crate::ipc::Message::Request {
                request: Request::RequestHelp { .. },
                ..
            }
        ));
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    });
    let (server_stream, client_stream) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let service = KeekServer { daemon }.serve(server_stream).await.unwrap();
        let _ = service.waiting().await;
    });
    let (read, mut write) = tokio::io::split(client_stream);
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"1\"}}}\n").await.unwrap();
    let _ = lines.next_line().await.unwrap().unwrap();
    write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"request_help\",\"arguments\":{\"session\":\"app_12-00-00\",\"reason\":\"2FA code\"},\"_meta\":{\"progressToken\":\"fixture-token\"}}}\n").await.unwrap();
    started_rx.await.unwrap();
    tokio::time::advance(Duration::from_secs(60)).await;
    let line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["method"], "notifications/progress");
    assert_eq!(value["params"]["progressToken"], "fixture-token");
    assert_eq!(value["params"]["progress"], 1.0);
    assert!(value["params"].get("message").is_none());
    server.abort();
    fake.abort();
}

#[tokio::test(start_paused = true)]
async fn progress_ticks_each_minute_only_with_a_token() {
    use std::sync::{Arc, Mutex};
    for token in [
        None,
        Some(rmcp::model::ProgressToken(
            rmcp::model::NumberOrString::String("wait".into()),
        )),
    ] {
        let values = Arc::new(Mutex::new(Vec::new()));
        let captured = values.clone();
        let enabled = token.is_some();
        let task = tokio::spawn(async move {
            waiting_with_progress(
                async {
                    tokio::time::sleep(Duration::from_secs(181)).await;
                    42
                },
                token,
                move |p| {
                    captured.lock().unwrap().push(p);
                    async {}
                },
            )
            .await
        });
        tokio::task::yield_now().await;
        for i in 1..=3 {
            tokio::time::advance(Duration::from_secs(60)).await;
            tokio::task::yield_now().await;
            assert_eq!(values.lock().unwrap().len(), if enabled { i } else { 0 });
        }
        let notifications = values.lock().unwrap().clone();
        assert_eq!(notifications.len(), if enabled { 3 } else { 0 });
        for (i, p) in notifications.iter().enumerate() {
            assert_eq!(p.progress, (i + 1) as f64);
            assert_eq!(p.message, None);
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(task.await.unwrap(), 42);
        tokio::time::advance(Duration::from_secs(120)).await;
        assert_eq!(values.lock().unwrap().len(), notifications.len());
    }
}
#[test]
fn request_help_requires_a_nonempty_reason_and_is_advertised() {
    assert!(own_tools().iter().any(|tool| tool.name == "request_help"));
    for reason in ["", " "] {
        assert!(parse_help(json!({"session":"app_12-00-00","reason":reason})).is_err());
    }
    assert!(parse_help(json!({"session":"app_12-00-00","reason":"2FA code"})).is_ok());
}
#[test]
fn help_reason_fits_one_line_and_preserves_session() {
    let Request::RequestHelp { session, reason } =
        parse_help(json!({"session":"app_12-00-00","reason":"2FA code"})).unwrap()
    else {
        panic!("wrong request")
    };
    assert_eq!(session.to_string(), "app_12-00-00");
    assert_eq!(reason, "2FA code");
    for reason in ["a".repeat(41), "two\nlines".into()] {
        assert!(parse_help(json!({"session":"app_12-00-00","reason":reason})).is_err());
    }
    assert!(parse_help(json!({"reason":"2FA code"})).is_err());
}
#[tokio::test(start_paused = true)]
async fn an_ordinary_call_finishes_without_progress() {
    let mut notifications = Vec::new();
    let result = waiting_with_progress(
        async { 42 },
        Some(rmcp::model::ProgressToken(
            rmcp::model::NumberOrString::String("fast".into()),
        )),
        |p| {
            notifications.push(p);
            async {}
        },
    )
    .await;
    assert_eq!(result, 42);
    assert!(notifications.is_empty());
}
#[tokio::test(start_paused = true)]
async fn a_daemon_reply_finishes_even_when_progress_delivery_stalls() {
    let task = tokio::spawn(waiting_with_progress(
        async {
            tokio::time::sleep(Duration::from_secs(61)).await;
            42
        },
        Some(rmcp::model::ProgressToken(
            rmcp::model::NumberOrString::String("wait".into()),
        )),
        |_| std::future::pending::<()>(),
    ));
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let finished = task.is_finished();
    if !finished {
        task.abort();
    }
    let result = task.await;
    assert!(finished, "daemon reply was held behind progress delivery");
    assert_eq!(result.unwrap(), 42);
}

#[test]
fn help_reason_limit_counts_characters_and_matches_schema() {
    let tool = own_tools()
        .into_iter()
        .find(|tool| tool.name == "request_help")
        .unwrap();
    assert_eq!(
        tool.input_schema["properties"]["reason"]["maxLength"],
        HELP_REASON_MAX_CHARS
    );
    assert!(parse_help(json!({"session":"app_12-00-00", "reason":"ü".repeat(40)})).is_ok());
    assert!(parse_help(json!({"session":"app_12-00-00", "reason":"ü".repeat(41)})).is_err());
}

#[test]
fn only_tools_that_can_wait_for_the_developer_get_progress() {
    let session: crate::session_folder::SessionName = "app_12-00-00".parse().unwrap();
    for (request, expected) in [
        (
            Request::RequestHelp {
                session: session.clone(),
                reason: "help".into(),
            },
            true,
        ),
        (
            Request::CallTool {
                session: session.clone(),
                tool: "click".into(),
                arguments: json!({}),
            },
            true,
        ),
        (
            Request::CloseSession {
                session: session.clone(),
                summary: "done".into(),
            },
            true,
        ),
        (Request::Status, false),
        (
            Request::OpenSession {
                project: "app".into(),
                worktree: "/tmp".into(),
                goal: "test".into(),
                checks: vec![],
            },
            false,
        ),
        (
            Request::AddCheck {
                session: session.clone(),
                text: "check".into(),
            },
            false,
        ),
        (
            Request::MarkCheck {
                session,
                check: crate::timeline::CheckId(1),
                outcome: crate::timeline::CheckOutcome::Passed,
                comment: "".into(),
            },
            false,
        ),
    ] {
        assert_eq!(can_wait(&request), expected, "{request:?}");
    }
}
