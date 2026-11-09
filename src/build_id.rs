use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BuildId(pub u64);

impl std::fmt::Display for BuildId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let timestamp: chrono::DateTime<chrono::Utc> =
            (UNIX_EPOCH + Duration::from_nanos(self.0)).into();
        f.write_str(&timestamp.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
    }
}

pub async fn current() -> Option<BuildId> {
    static BUILD: OnceCell<Option<BuildId>> = OnceCell::const_new();
    *BUILD.get_or_init(|| async {
        let read = tokio::task::spawn_blocking(|| {
            // /proc follows the running inode even after cargo install replaces its path.
            let modified = std::fs::metadata("/proc/self/exe")?.modified()?;
            let elapsed = modified.duration_since(UNIX_EPOCH).map_err(std::io::Error::other)?;
            u64::try_from(elapsed.as_nanos()).map(BuildId).map_err(std::io::Error::other)
        }).await;
        match read {
            Ok(Ok(build)) => Some(build),
            error => {
                tracing::warn!(?error, "Cannot read this keek build's modification time; comparing daemon versions only");
                None
            }
        }
    }).await
}
