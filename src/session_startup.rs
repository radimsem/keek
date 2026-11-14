use crate::session::SessionError;
use std::path::Path;
use std::time::Duration;

pub(super) async fn wait_for_cover(
    folder: &Path,
    limit: Duration,
    poll: Duration,
) -> Result<(), SessionError> {
    let marker = folder.join("cover-clear");
    let clear = async {
        loop {
            let marker = marker.clone();
            if tokio::task::spawn_blocking(move || marker.exists())
                .await
                .unwrap_or(false)
            {
                return;
            }
            tokio::time::sleep(poll).await;
        }
    };
    tokio::time::timeout(limit, clear).await.map_err(|_| {
        SessionError::Refused(format!(
            "Omarchy's startup cover still hid the nested desktop after {} s. Open again; if it repeats, check the Omarchy version against docs/facts.md.",
            limit.as_secs_f64()
        ))
    })
}

const TAIL_BYTES: u64 = 2048;

/// `reason` is one or more full sentences that end with what to do next.
pub(super) async fn startup_refusal(folder: &Path, reason: &str) -> SessionError {
    let folder = folder.to_owned();
    let logs = tokio::task::spawn_blocking(move || {
        let mut logs = String::new();
        // The folder is removed once the open fails, so the refusal quotes the logs.
        for name in ["sandbox.err", "session.err", "hyprland.log"] {
            if let Ok(bytes) =
                crate::session_folder::read_sandbox_file(&folder.join(name), Some(TAIL_BYTES))
                && !bytes.is_empty()
            {
                let tail = String::from_utf8_lossy(&bytes);
                // A cut first line could hold half a Test Credential, which masking would miss.
                let tail = if bytes.len() as u64 == TAIL_BYTES {
                    tail.split_once('\n').map_or("", |(_, rest)| rest)
                } else {
                    &tail
                };
                if tail.is_empty() {
                    continue;
                }
                let lines = tail.lines().rev().take(20).collect::<Vec<_>>();
                logs.push_str(&format!("\n{name}:\n"));
                logs.push_str(&lines.into_iter().rev().collect::<Vec<_>>().join("\n"));
            }
        }
        logs
    })
    .await
    .unwrap_or_default();
    let logs = if logs.is_empty() {
        logs
    } else {
        format!(" Its last log lines:{logs}")
    };
    SessionError::Refused(format!(
        "Agent Session desktop did not start. {reason}{logs}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_cover_wait_returns_when_the_clear_marker_appears() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("cover-clear");
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            std::fs::write(marker, "").unwrap();
        });
        let result = wait_for_cover(
            directory.path(),
            Duration::from_secs(1),
            Duration::from_millis(5),
        )
        .await;
        let existed = directory.path().join("cover-clear").exists();
        writer.await.unwrap();
        result.unwrap();
        assert!(existed, "returned before the cover cleared");
    }

    #[tokio::test]
    async fn the_cover_wait_refuses_when_the_desktop_stays_hidden() {
        let directory = tempfile::tempdir().unwrap();
        let start = tokio::time::Instant::now();
        let limit = Duration::from_millis(30);
        let error = wait_for_cover(directory.path(), limit, Duration::from_millis(5))
            .await
            .expect_err("a hidden desktop must be refused")
            .to_string();
        assert!(start.elapsed() >= limit);
        assert!(
            error.contains("cover") && error.contains("Omarchy"),
            "{error}"
        );
        assert!(
            error.contains("Open again") && error.contains("docs/facts.md"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_startup_refusal_quotes_the_log_tail_and_names_only_existing_logs() {
        let directory = tempfile::tempdir().unwrap();
        let lines = (0..10_000)
            .map(|n| format!("sandbox line {n}\n"))
            .collect::<String>();
        std::fs::write(directory.path().join("sandbox.err"), lines).unwrap();
        let error = startup_refusal(directory.path(), "The sandbox stopped. Open again.")
            .await
            .to_string();
        assert!(error.contains("sandbox line 9999"), "{error}");
        assert!(error.contains("sandbox line 9980"), "{error}");
        assert!(!error.contains("sandbox line 9979"), "{error}");
        assert!(
            error.starts_with("Agent Session desktop did not start. The sandbox stopped."),
            "{error}"
        );
        assert!(error.contains("sandbox.err:"), "{error}");
        for absent in ["session.err", "hyprland.log", "driver.log"] {
            assert!(!error.contains(absent), "{error}");
        }
    }
}
