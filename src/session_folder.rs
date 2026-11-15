use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// An Agent Session's name, such as `myapp_14-03-22`. It is safe to use as one
/// path component, because the Project part comes from the coding agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionName(String);

impl TryFrom<String> for SessionName {
    type Error = InvalidSessionName;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        name.parse()
    }
}

impl From<SessionName> for String {
    fn from(name: SessionName) -> Self {
        name.0
    }
}

/// One Action Log entry's number within its Agent Session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Turn(pub u32);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0:?} is not a valid session name, because it can't be one folder name")]
pub struct InvalidSessionName(String);

impl FromStr for SessionName {
    type Err = InvalidSessionName;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
            return Err(InvalidSessionName(name.to_owned()));
        }
        Ok(Self(name.to_owned()))
    }
}

impl fmt::Display for SessionName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The paths inside `~/.cache/keek/sessions/<session>/`, where Cua Driver records
/// and the daemon writes the Session Timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionFolder {
    root: PathBuf,
}

impl SessionFolder {
    #[must_use]
    pub fn new(sessions_dir: impl AsRef<Path>, session: &SessionName) -> Self {
        Self {
            root: sessions_dir.as_ref().join(&session.0),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn recording(&self) -> PathBuf {
        self.root.join("recording.mp4")
    }

    #[must_use]
    pub fn timeline(&self) -> PathBuf {
        self.root.join("timeline.json")
    }

    #[must_use]
    pub fn turn(&self, turn: Turn) -> PathBuf {
        self.root.join(format!("turn-{:05}", turn.0))
    }

    /// The file Cua Driver writes for one Action.
    #[must_use]
    pub fn action(&self, turn: Turn) -> PathBuf {
        self.turn(turn).join("action.json")
    }
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)
}

pub(crate) fn read_sandbox_file(path: &Path, tail: Option<u64>) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    let limit = tail.unwrap_or(64 * 1024).min(64 * 1024);
    if tail.is_some() {
        file.seek(SeekFrom::Start(
            file.metadata()?.len().saturating_sub(limit),
        ))?;
    }
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}
