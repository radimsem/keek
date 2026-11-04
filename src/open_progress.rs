//! Progress messages for a slow open: the phase names and the latest install log line.

use crate::credentials::Credentials;
use crate::ipc::Event;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;

/// The newest log line goes out at most this often.
const LOG_INTERVAL: Duration = Duration::from_secs(10);
const LINE_MAX_CHARS: usize = 200;
const TAIL_BYTES: u64 = 4096;

/// Where an open reports progress. The default sink drops every message.
#[derive(Clone, Default)]
pub struct OpenProgress {
    events: Option<mpsc::Sender<Event>>,
    credentials: Credentials,
}

impl OpenProgress {
    pub fn new(events: Option<mpsc::Sender<Event>>, credentials: Credentials) -> Self {
        Self {
            events,
            credentials,
        }
    }

    /// Masks `message` and drops it when the queue is full, so a slow client never stalls an open.
    pub fn send(&self, message: &str) {
        if let Some(events) = &self.events {
            let _ = events.try_send(Event::Progress {
                message: self.credentials.mask(message),
            });
        }
    }

    /// Runs `work` and, while it runs, sends the newest finished line of `log` every 10 seconds,
    /// prefixed `install: `, unless it is the line already sent.
    pub async fn tailing<T>(&self, log: &Path, work: impl Future<Output = T>) -> T {
        if self.events.is_none() {
            return work.await;
        }
        tokio::pin!(work);
        let mut ticker =
            tokio::time::interval_at(tokio::time::Instant::now() + LOG_INTERVAL, LOG_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut sent: Option<String> = None;
        loop {
            tokio::select! {
                biased;
                output = &mut work => return output,
                _ = ticker.tick() => {
                    let read = tokio::task::spawn_blocking({
                        let log = log.to_owned();
                        move || last_line(&log)
                    });
                    let line = tokio::select! {
                        biased;
                        output = &mut work => return output,
                        line = read => line.ok().flatten(),
                    };
                    // Masking comes before the cut so the cut can't leave half a value.
                    if let Some(line) = line.filter(|line| sent.as_ref() != Some(line)) {
                        self.send(&format!("install: {}", cap(&self.credentials.mask(&line))));
                        sent = Some(line);
                    }
                }
            }
        }
    }
}

fn cap(line: &str) -> String {
    line.chars().take(LINE_MAX_CHARS).collect()
}

/// The newest non-empty line that the writer has finished. The last 4 KiB is enough: a line that
/// starts before the window, or has no line end yet, could hold half of a credential value that
/// masking would not recognize.
fn last_line(log: &Path) -> Option<String> {
    let mut file = std::fs::File::open(log).ok()?;
    let start = file.metadata().ok()?.len().saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.split(['\n', '\r']).collect();
    lines.pop();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines
        .into_iter()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "open_progress_tests.rs"]
mod tests;
