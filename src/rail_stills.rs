use crate::compositor::WindowAddress;
use crate::process::{ProcessError, ProcessRunner};
use crate::session_folder::SessionName;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub const STILL_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct StillSchedule {
    last: HashMap<SessionName, Instant>,
}

impl StillSchedule {
    pub(crate) fn forget(&mut self, name: &SessionName) {
        self.last.remove(name);
    }

    pub fn due(&mut self, now: Instant, rail: &[SessionName]) -> Vec<SessionName> {
        self.last.retain(|name, _| rail.contains(name));
        let mut due = Vec::new();
        for name in rail {
            if self
                .last
                .get(name)
                .is_none_or(|last| now.duration_since(*last) >= STILL_INTERVAL)
            {
                self.last.insert(name.clone(), now);
                due.push(name.clone());
            }
        }
        due
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StillError {
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error(
        "Could not read Session Windows from Hyprland: {0}. Check hyprctl clients -j and retry."
    )]
    Clients(#[from] serde_json::Error),
    #[error(
        "Session Window {0} has no stable ID in Hyprland. Check that the Session Window is still open."
    )]
    MissingWindow(String),
    #[error(
        "Could not save the rail still at {0} because its path is not UTF-8. Use a UTF-8 runtime directory."
    )]
    InvalidPath(PathBuf),
    #[error("Could not save the rail still at {0}: {1}. Check the frames folder permissions.")]
    Save(PathBuf, std::io::Error),
    #[error("Could not finish saving the rail still: {0}. Retry the capture.")]
    SaveTask(#[from] tokio::task::JoinError),
}

pub fn still_path(frames_dir: &Path, name: &SessionName) -> PathBuf {
    frames_dir.join(format!("{name}-still.jpg"))
}

pub(crate) async fn take(
    runner: &dyn ProcessRunner,
    window: &WindowAddress,
    scale: f64,
    path: &Path,
    complete: Arc<oneshot::Sender<()>>,
) -> Result<(), StillError> {
    let clients = runner.output("hyprctl", &["clients", "-j"], None).await?;
    let clients: Vec<serde_json::Value> = serde_json::from_str(&clients)?;
    let stable_id = clients
        .iter()
        .find(|client| client["address"].as_str() == Some(window.0.as_str()))
        .and_then(|client| client["stableId"].as_str())
        .ok_or_else(|| StillError::MissingWindow(window.0.clone()))?;
    let next = path.with_extension("next.jpg");
    let destination = next
        .to_str()
        .ok_or_else(|| StillError::InvalidPath(next.clone()))?;
    runner
        .output(
            "grim",
            &[
                "-T",
                stable_id,
                "-t",
                "jpeg",
                "-q",
                "70",
                "-s",
                &format!("{scale:.3}"),
                destination,
            ],
            None,
        )
        .await?;
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let result = std::fs::rename(next, &path).map_err(|error| StillError::Save(path, error));
        // Dismissal waits for both the capture task and any rename already in progress.
        drop(complete);
        result
    })
    .await?
}

#[cfg(test)]
#[path = "rail_stills_tests.rs"]
mod tests;
