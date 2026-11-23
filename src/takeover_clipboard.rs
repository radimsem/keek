use crate::process::{ProcessError, ProcessRunner};
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant};

pub const FIFO_NAME: &str = "clipboard.fifo";

const HOST_READ_TIMEOUT: Duration = Duration::from_secs(2);
const FIFO_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const TEXT_TYPES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

pub struct CarriedText(Vec<u8>);

impl CarriedText {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for CarriedText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "CarriedText(<{} bytes>)", self.0.len())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error(
        "Can't read the host clipboard: {0}. Paste is unavailable in this Takeover, so type the text instead."
    )]
    HostRead(String),
    #[error(
        "Can't reach the session's clipboard: {0}. Paste is unavailable in this Takeover, so type the text instead."
    )]
    Nested(std::io::Error),
}

pub type ClipboardFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ClipboardError>> + Send + 'a>>;

/// The session's own clipboard. It is only ever written, never read back to the host.
pub trait NestedClipboard: Send + Sync {
    fn copy(&self, text: CarriedText) -> ClipboardFuture<'_>;
    fn clear(&self) -> ClipboardFuture<'_>;
}

/// Reads the host clipboard's plain text. `None` when it holds no text type or nothing.
pub async fn read_host_text(
    runner: &dyn ProcessRunner,
) -> Result<Option<CarriedText>, ClipboardError> {
    let timed_out = |_| ClipboardError::HostRead("wl-paste timed out".into());
    let types = tokio::time::timeout(
        HOST_READ_TIMEOUT,
        runner.output("wl-paste", &["--list-types"], None),
    )
    .await
    .map_err(timed_out)?;
    let types = match types {
        Ok(types) => types,
        Err(ProcessError::Failed { message, .. }) if message.contains("Nothing is copied") => {
            return Ok(None);
        }
        Err(error) => return Err(ClipboardError::HostRead(error.to_string())),
    };
    let Some(kind) = TEXT_TYPES
        .into_iter()
        .find(|kind| types.lines().any(|line| line == *kind))
    else {
        return Ok(None);
    };
    let text = tokio::time::timeout(
        HOST_READ_TIMEOUT,
        runner.output_bytes("wl-paste", &["--no-newline", "--type", kind], None),
    )
    .await
    .map_err(timed_out)?
    .map(CarriedText::new)
    .map_err(|error| {
        // HostRunner can put clipboard stdout in a process error.
        let message = match error {
            ProcessError::Timeout { .. } | ProcessError::Stalled { .. } => "wl-paste timed out",
            ProcessError::Io { .. } => "could not run wl-paste",
            ProcessError::Failed { .. } => "wl-paste exited with an error",
        };
        ClipboardError::HostRead(message.into())
    })?;
    Ok((!text.as_bytes().is_empty()).then_some(text))
}

/// Writes frames to the FIFO that `assets/nested/clipboard.sh` reads inside the sandbox.
pub struct SessionFifo {
    path: PathBuf,
}

impl SessionFifo {
    pub fn new(session_dir: &Path) -> Self {
        Self {
            path: session_dir.join(FIFO_NAME),
        }
    }

    async fn write_frame(&self, text: Option<CarriedText>) -> Result<(), ClipboardError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let frame = match &text {
                Some(text) => {
                    let bytes = text.as_bytes();
                    let mut frame = format!("copy {}\n", bytes.len()).into_bytes();
                    frame.extend_from_slice(bytes);
                    frame
                }
                None => b"clear\n".to_vec(),
            };
            write_fifo(&path, &frame)
        })
        .await
        .map_err(|error| ClipboardError::Nested(io::Error::other(error)))?
        .map_err(ClipboardError::Nested)
    }
}

impl NestedClipboard for SessionFifo {
    fn copy(&self, text: CarriedText) -> ClipboardFuture<'_> {
        Box::pin(self.write_frame(Some(text)))
    }
    fn clear(&self) -> ClipboardFuture<'_> {
        Box::pin(self.write_frame(None))
    }
}

fn write_fifo(path: &Path, mut frame: &[u8]) -> io::Result<()> {
    let mut fifo = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    if !fifo.metadata()?.file_type().is_fifo() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard path is not a FIFO",
        ));
    }
    let deadline = Instant::now() + FIFO_WRITE_TIMEOUT;
    while !frame.is_empty() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "writing to the clipboard FIFO timed out",
            ));
        }
        match fifo.write(frame) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => frame = &frame[written..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Creates the session's clipboard FIFO, readable and writable by the developer only.
pub fn create_fifo(session_dir: &Path) -> io::Result<()> {
    let path = session_dir.join(FIFO_NAME);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) if error.kind() == io::ErrorKind::IsADirectory => fs::remove_dir(&path)?,
        Err(error) => return Err(error),
    }
    let name = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: name keeps the NUL-terminated path alive for mkfifo.
    if unsafe { libc::mkfifo(name.as_ptr(), 0o600) } != 0 {
        return Err(io::Error::last_os_error());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
#[path = "takeover_clipboard_tests.rs"]
mod tests;
