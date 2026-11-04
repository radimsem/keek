use super::*;
use crate::ipc::{Connection, Message, RequestId};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Calls `tool` through the MCP server, cancels the call, and returns what the
/// daemon saw next on the wire after the request itself.
async fn daemon_view_after_cancel(tool: &str, arguments: &str) -> (RequestId, Option<Message>) {
    let (daemon_stream, fake_stream) = tokio::io::duplex(4096);
    let daemon = DaemonClient::new(Connection::new(daemon_stream));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let fake = tokio::spawn(async move {
        let mut connection = Connection::new(fake_stream);
        let Some(Message::Request { id, .. }) = connection.recv().await.unwrap() else {
            panic!("the daemon got no request");
        };
        let _ = started_tx.send(());
        let next = tokio::time::timeout(Duration::from_secs(2), connection.recv())
            .await
            .ok()
            .map(|message| message.unwrap());
        (id, next.flatten())
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
    let call = format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}\n"
    );
    write.write_all(call.as_bytes()).await.unwrap();
    started_rx.await.unwrap();
    write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":2,\"reason\":\"user pressed Esc\"}}\n").await.unwrap();
    let seen = fake.await.unwrap();
    server.abort();
    seen
}

#[tokio::test(start_paused = true)]
async fn cancelling_request_help_withdraws_the_request_at_the_daemon() {
    let (id, next) = daemon_view_after_cancel(
        "request_help",
        r#"{"session":"app_12-00-00","reason":"2FA code"}"#,
    )
    .await;

    assert_eq!(next, Some(Message::Cancel { id }));
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_forwarded_call_leaves_it_running_at_the_daemon() {
    let (_, next) = daemon_view_after_cancel("click", r#"{"session":"app_12-00-00"}"#).await;

    assert_eq!(next, None);
}
