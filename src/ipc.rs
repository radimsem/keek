//! The protocol between `keek mcp` and `keek daemon`: newline-delimited JSON over
//! a Unix socket. The client opens with `Hello`, the daemon answers with its own,
//! and then requests, responses and events follow, keyed by the request's id.
//!
//! `Hello` and `Shutdown` must keep their shape across releases, because they are how
//! a new `keek mcp` replaces an older daemon.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello { version: String },
    Request { id: RequestId, request: Request },
    Response { id: RequestId, response: Response },
    Event { id: RequestId, event: Event },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Request {
    Status,
    /// Asks the daemon to exit. It refuses while an Agent Session is open.
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Response {
    Status(DaemonStatus),
    ShuttingDown,
    ShutdownRefused { open_sessions: usize },
}

/// Something a long-running request reports before its response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Event {
    Progress { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatus {
    pub version: String,
    pub pid: u32,
    pub open_sessions: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("the connection failed")]
    Io(#[from] std::io::Error),
    #[error("the message is not valid")]
    Json(#[from] serde_json::Error),
}

/// One end of the protocol over any byte stream, so tests can use an in-memory
/// duplex in place of the socket.
#[derive(Debug)]
pub struct Connection<S> {
    stream: BufReader<S>,
    line: String,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Connection<S> {
    #[must_use]
    pub fn new(stream: S) -> Self {
        Self {
            stream: BufReader::new(stream),
            line: String::new(),
        }
    }

    /// Writes one message and flushes it.
    ///
    /// # Errors
    ///
    /// Fails when the peer has gone or the message can't be serialized.
    pub async fn send(&mut self, message: &Message) -> Result<(), IpcError> {
        let mut bytes = serde_json::to_vec(message)?;
        bytes.push(b'\n');
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Reads the next message. `None` means the peer closed the connection.
    /// Not cancel safe: dropping this future can lose a partly read line.
    ///
    /// # Errors
    ///
    /// Fails when the read fails or the line isn't a valid message.
    pub async fn recv(&mut self) -> Result<Option<Message>, IpcError> {
        self.line.clear();
        if self.stream.read_line(&mut self.line).await? == 0 {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&self.line)?))
    }
}
