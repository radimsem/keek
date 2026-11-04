//! One client connection: the Hello exchange, then requests in and replies out,
//! where replies may come back in any order.

use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

use crate::VERSION;
use crate::ipc::{Connection, IpcError, Message, Request, Response};

const REQUEST_QUEUE: usize = 32;

pub(super) struct PendingRequest {
    pub(super) opener: tokio::sync::watch::Receiver<()>,
    pub(super) request: Request,
    pub(super) reply: oneshot::Sender<Response>,
    pub(super) events: Option<mpsc::Sender<crate::ipc::Event>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectionEnd {
    ClientLeft,
    ShutdownAccepted,
}

pub(super) async fn serve_client(
    stream: UnixStream,
    requests: mpsc::Sender<PendingRequest>,
) -> ConnectionEnd {
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
    let (_alive, opener) = tokio::sync::watch::channel(());
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
                requests.send(PendingRequest { opener: opener.clone(), request, reply, events: watches.then_some(events) }).await.map_err(|_| ConnectionError::OwnerGone)?;
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::duplex;

    use super::*;
    use crate::ipc::{Event, RequestId};

    #[tokio::test]
    async fn disconnect_signals_the_connections_requests() {
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
        let pending = requests.recv().await.unwrap();
        assert!(pending.opener.has_changed().is_ok());
        drop(client);
        assert_eq!(server.await.unwrap().unwrap(), ConnectionEnd::ClientLeft);
        assert!(pending.opener.has_changed().is_err());
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
