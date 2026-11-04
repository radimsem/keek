//! The protocol between `keek mcp` and `keek daemon`: newline-delimited JSON over
//! a Unix socket. The client opens with `Hello`, the daemon answers with its own,
//! and then requests, responses and events follow, keyed by the request's id.
//!
//! `Hello` and `Shutdown` must keep their shape across releases, because they are how
//! a new `keek mcp` replaces an older daemon.

use crate::checklist::Mark;
use crate::session_folder::SessionName;
use crate::timeline::{CheckId, CheckOutcome, Millis};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello {
        version: String,
    },
    Request {
        id: RequestId,
        request: Request,
    },
    Response {
        id: RequestId,
        response: Response,
    },
    Event {
        id: RequestId,
        event: Event,
    },
    /// The client no longer wants the answer to the request with this id.
    Cancel {
        id: RequestId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Request {
    Status,
    ListTools,
    OpenSession {
        project: String,
        worktree: PathBuf,
        goal: String,
        checks: Vec<String>,
    },
    CallTool {
        session: SessionName,
        tool: String,
        arguments: Value,
    },
    RequestHelp {
        session: SessionName,
        reason: String,
    },
    AddCheck {
        session: SessionName,
        text: String,
    },
    MarkCheck {
        session: SessionName,
        check: CheckId,
        outcome: CheckOutcome,
        comment: String,
    },
    CloseSession {
        session: SessionName,
        summary: String,
    },
    WatchSession {
        session: SessionName,
    },
    SetProjectProfile {
        project: String,
        profile: Value,
        credentials: Option<CredentialSource>,
    },
    /// Asks the daemon to exit. It refuses while an Agent Session is open.
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Response {
    Status(DaemonStatus),
    Tools(Value),
    Opened {
        credentials: Vec<String>,
        session: SessionName,
        checks: Vec<OpenedCheck>,
        warnings: Vec<String>,
    },
    CheckAdded {
        check: CheckId,
    },
    CheckMarked {
        check: CheckId,
    },
    ToolResult(Value),
    Closed(Value),
    Refused {
        message: String,
    },
    WatchEnded,
    ShuttingDown,
    ShutdownRefused {
        open_sessions: usize,
    },
    ProfileWritten {
        path: PathBuf,
        profile: String,
        credentials: Option<crate::project::CopiedCredentials>,
    },
}

/// The Test Credentials a profile write copies, and the host file holding their values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialSource {
    pub source: PathBuf,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedCheck {
    pub id: CheckId,
    pub text: String,
}

/// Everything a live view shows about one Agent Session. Each publish replaces the
/// last value whole, so a watcher that misses one still sees every Check's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    #[serde(default)]
    pub control: ControlState,
    pub state: SessionState,
    pub checks: Vec<CheckStatus>,
    pub last_action: Option<LastAction>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckStatus {
    pub id: CheckId,
    pub text: String,
    pub mark: Mark,
    #[serde(default)]
    pub added: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastAction {
    pub tool: String,
    // Time since the Agent Session opened, independent of the recording lead.
    pub time_ms: Millis,
}

/// Something a long-running request reports before its response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Event {
    Progress { message: String },
    Snapshot(SessionSnapshot),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Opening,
    Open,
    Closing,
    Rendering {
        percent: Percent,
        frame: Option<PathBuf>,
        elapsed: Millis,
    },
    Ended(Box<EndedSession>),
}

impl SessionState {
    pub(crate) fn ticks(&self) -> bool {
        match self {
            Self::Opening | Self::Open | Self::Closing => true,
            Self::Rendering { .. } | Self::Ended(_) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatus {
    pub version: String,
    pub pid: u32,
    pub open_sessions: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("the connection failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("the message is not valid: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct Connection<S> {
    reader: ConnectionReader<tokio::io::ReadHalf<S>>,
    writer: ConnectionWriter<tokio::io::WriteHalf<S>>,
}

#[derive(Debug)]
pub struct ConnectionReader<R> {
    stream: BufReader<R>,
    line: String,
}

#[derive(Debug)]
pub struct ConnectionWriter<W> {
    stream: W,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Connection<S> {
    #[must_use]
    pub fn new(stream: S) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            reader: ConnectionReader {
                stream: BufReader::new(reader),
                line: String::new(),
            },
            writer: ConnectionWriter { stream: writer },
        }
    }

    pub fn split(
        self,
    ) -> (
        ConnectionReader<tokio::io::ReadHalf<S>>,
        ConnectionWriter<tokio::io::WriteHalf<S>>,
    ) {
        (self.reader, self.writer)
    }

    pub async fn send(&mut self, message: &Message) -> Result<(), IpcError> {
        self.writer.send(message).await
    }

    pub async fn recv(&mut self) -> Result<Option<Message>, IpcError> {
        self.reader.recv().await
    }
}

impl<R: AsyncRead + Unpin> ConnectionReader<R> {
    // Dropping this future can lose a partly read line; keep it in a reader task.
    pub async fn recv(&mut self) -> Result<Option<Message>, IpcError> {
        self.line.clear();
        if self.stream.read_line(&mut self.line).await? == 0 {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&self.line)?))
    }
}

impl<W: AsyncWrite + Unpin> ConnectionWriter<W> {
    pub fn boxed(self) -> ConnectionWriter<Box<dyn AsyncWrite + Unpin + Send>>
    where
        W: Send + 'static,
    {
        ConnectionWriter {
            stream: Box::new(self.stream),
        }
    }

    pub async fn shutdown(&mut self) -> std::io::Result<()> {
        self.stream.shutdown().await
    }

    pub async fn send(&mut self, message: &Message) -> Result<(), IpcError> {
        let mut bytes = serde_json::to_vec(message)?;
        bytes.push(b'\n');
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlState {
    #[default]
    Verifier,
    HelpRequest {
        reason: String,
        start_ms: Millis,
    },
    Takeover {
        start_ms: Millis,
        shortcuts: HostShortcuts,
    },
    HandBack {
        end_ms: Millis,
        length: Millis,
    },
}
impl ControlState {
    pub(crate) fn pauses_idle(&self) -> bool {
        match self {
            Self::HelpRequest { .. } | Self::Takeover { .. } => true,
            Self::Verifier | Self::HandBack { .. } => false,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostShortcuts {
    Captured,
    Live,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_requests_cross_as_names_and_values() {
        let message = r#"{"type":"request","id":8,"request":{"kind":"call_tool","session":"app_12-00-00","tool":"click","arguments":{"x":10}}}"#;
        let parsed = serde_json::from_str::<Message>(message);
        assert!(parsed.is_ok(), "session call was refused: {parsed:?}");
    }
    #[test]
    fn ipc_error_includes_its_source() {
        let error = IpcError::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "fixture broken pipe",
        ));
        assert!(error.to_string().contains("fixture broken pipe"));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Percent(pub u8);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndedSession {
    pub end: crate::checklist::SessionEnd,
    pub revision: crate::revision::Revision,
    pub attempt: Option<crate::attempts::Attempt>,
    pub verification: Option<PathBuf>,
    pub folder: Option<PathBuf>,
    pub video_length: Millis,
    pub actions: usize,
    pub frame: Option<PathBuf>,
    pub elapsed: Millis,
}
