use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;

/// A time in milliseconds from the start of the Raw Recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Millis(pub u64);

/// A Check's stable id within its Agent Session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CheckId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineEvent {
    /// One forwarded tool call. `action` is true when the tool isn't marked
    /// `readOnlyHint`, so one `run_steps` call is one active span.
    Call {
        tool: String,
        action: bool,
        start_ms: Millis,
        end_ms: Millis,
        is_error: bool,
    },
    CheckMark {
        check: CheckId,
        outcome: CheckOutcome,
        at_ms: Millis,
    },
    Takeover {
        reason: String,
        start_ms: Millis,
        end_ms: Millis,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTimeline {
    schema_version: u32,
    video_lead_ms: Millis,
    events: Vec<TimelineEvent>,
}

#[derive(Debug, thiserror::Error)]
pub enum TimelineError {
    #[error("the Session Timeline is not valid JSON for this schema")]
    Json(#[from] serde_json::Error),
    #[error("Session Timeline schema version {0} is not supported")]
    UnsupportedSchema(u32),
}

impl SessionTimeline {
    /// `video_lead` is how long wf-recorder's first frame lands after `start_recording`
    /// is forwarded. It is recorded for reference: every stored time has already had it
    /// subtracted, so readers must not subtract it again.
    #[must_use]
    pub fn new(video_lead: Millis) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            video_lead_ms: video_lead,
            events: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, TimelineError> {
        #[derive(Deserialize)]
        struct Version {
            schema_version: u32,
        }

        let Version { schema_version } = serde_json::from_str(text)?;
        if schema_version != SCHEMA_VERSION {
            return Err(TimelineError::UnsupportedSchema(schema_version));
        }
        Ok(serde_json::from_str(text)?)
    }

    #[must_use]
    pub fn video_lead(&self) -> Millis {
        self.video_lead_ms
    }

    #[must_use]
    pub fn events(&self) -> &[TimelineEvent] {
        &self.events
    }

    pub fn push(&mut self, event: TimelineEvent) {
        self.events.push(event);
    }
}
