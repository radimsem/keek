use crate::{
    checklist::Check,
    revision::Revision,
    session_folder::SessionName,
    timeline::{Millis, SessionTimeline, TimelineEvent},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
pub struct Journal {
    pub project: String,
    pub name: SessionName,
    pub started: String,
    pub revision: Revision,
    pub goal: String,
    pub checks: Vec<Check>,
    pub timeline: SessionTimeline,
    pub takeovers: Vec<Takeover>,
    pub credentials_hash: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Takeover {
    pub start: Millis,
    pub end: Option<Millis>,
}
#[derive(Clone)]
pub struct Handle {
    pub path: PathBuf,
    pub recording_offset: Arc<std::sync::atomic::AtomicU64>,
    pub data: Arc<Mutex<Journal>>,
}
impl Handle {
    pub async fn create(
        directory: &Path,
        journal: Journal,
    ) -> Result<Self, crate::session::SessionError> {
        let handle = Self {
            recording_offset: Arc::new(std::sync::atomic::AtomicU64::new(
                crate::session::VIDEO_LEAD.0,
            )),
            path: directory.join(format!("{}.json", journal.name)),
            data: Arc::new(Mutex::new(journal)),
        };
        handle.save().await?;
        Ok(handle)
    }
    pub async fn save(&self) -> Result<(), crate::session::SessionError> {
        let data = self.data.lock().await;
        self.write(&data).await
    }
    pub async fn write(&self, data: &Journal) -> Result<(), crate::session::SessionError> {
        let path = self.path.clone();
        let bytes = serde_json::to_vec_pretty(data)?;
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
            let directory = path
                .parent()
                .ok_or_else(|| std::io::Error::other("journal has no folder"))?;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(directory)?;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
            let temporary = path.with_extension("tmp");
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            std::fs::rename(temporary, path)
        })
        .await
        .map_err(|e| crate::session::SessionError::Refused(e.to_string()))??;
        Ok(())
    }
    pub async fn remove(&self) {
        remove(self.path.clone()).await;
    }
}
pub async fn remove(path: PathBuf) {
    let result = tokio::task::spawn_blocking(move || std::fs::remove_file(path)).await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        error => tracing::warn!(?error, "could not remove session journal"),
    }
}
impl Journal {
    pub fn recovery_timeline(&self, recording_end: Millis) -> SessionTimeline {
        let mut timeline = self.timeline.clone();
        if timeline
            .events()
            .iter()
            .any(|event| matches!(event, TimelineEvent::Takeover { .. }))
        {
            return timeline;
        }
        for span in &self.takeovers {
            timeline.push(TimelineEvent::Takeover {
                reason: None,
                start_ms: span.start,
                end_ms: span.end.unwrap_or(recording_end),
            });
        }
        timeline
    }
}
