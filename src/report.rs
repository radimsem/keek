use crate::checklist::{Check, Mark, SessionEnd};
use crate::revision::Revision;
use crate::timeline::{CheckOutcome, Millis};

pub trait VideoTimeMap {
    fn video_time(&self, raw: Millis) -> Millis;
}
pub struct IdentityTimeMap;
impl VideoTimeMap for IdentityTimeMap {
    fn video_time(&self, raw: Millis) -> Millis {
        raw
    }
}
pub struct ReportInputs<'a> {
    pub session: &'a str,
    pub goal: &'a str,
    pub revision: &'a Revision,
    pub summary: &'a str,
    pub timeline_events: &'a [crate::timeline::TimelineEvent],
    pub checks: &'a [Check],
    pub end: &'a SessionEnd,
    pub time_map: &'a dyn VideoTimeMap,
    pub video_error: Option<&'a str>,
    pub actions_error: Option<&'a str>,
    pub profile: &'a ProfileSummary,
}

/// What built the session's environment, since a commit no longer says. `[env]` keeps only its names.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProfileSummary {
    pub install: String,
    pub start: String,
    pub toolchains: Vec<ToolVersion>,
    pub env: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolVersion {
    pub tool: String,
    pub version: String,
}

impl ProfileSummary {
    pub fn new(
        profile: &crate::project::Profile,
        toolchains: &crate::toolchain::Toolchains,
    ) -> Self {
        Self {
            install: profile.install.clone(),
            start: profile.start.clone(),
            toolchains: toolchains
                .tools
                .iter()
                .map(|tool| ToolVersion {
                    tool: tool.tool.name().to_owned(),
                    version: tool.version.clone(),
                })
                .collect(),
            env: profile.env.keys().cloned().collect(),
        }
    }

    pub fn masked(mut self, credentials: &crate::credentials::Credentials) -> Self {
        self.install = credentials.mask(&self.install);
        self.start = credentials.mask(&self.start);
        self
    }
}
// open_session takes the goal and Checks as free text, and a newline would break a heading,
// a list item or an index line.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}
// A code span whose fence is longer than any backtick run inside, as CommonMark requires.
fn code(text: &str) -> String {
    let text = one_line(text);
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    if longest == 0 {
        return format!("`{text}`");
    }
    let fence = "`".repeat(longest + 1);
    format!("{fence} {text} {fence}")
}
pub fn render(input: &ReportInputs<'_>) -> String {
    let revision = input.revision;
    let commit = revision.commit.get(..7).unwrap_or(&revision.commit);
    let branch = revision.branch.as_deref().unwrap_or("detached HEAD");
    let dirty = if revision.dirty {
        " + uncommitted changes"
    } else {
        ""
    };
    let mut text = format!(
        "# {}\n\nVerified {branch} at {commit}{dirty}\n\nAgent Session: {}\n\n## Verification Checklist\n\n",
        one_line(input.goal),
        input.session
    );
    for check in input.checks {
        let added = if check.added { " (added)" } else { "" };
        let mark = match &check.mark {
            Mark::Unmarked => "Not marked".into(),
            Mark::Outcome {
                outcome,
                comment,
                at,
            } => {
                let outcome = match outcome {
                    CheckOutcome::Passed => "Passed",
                    CheckOutcome::Failed => "Failed",
                    CheckOutcome::Skipped => "Skipped",
                };
                let seconds = input.time_map.video_time(*at).0 / 1000;
                let comment = if comment.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {comment}")
                };
                format!("{outcome} at {}:{:02}{comment}", seconds / 60, seconds % 60)
            }
        };
        text.push_str(&format!(
            "- {}. {}{added}: {mark}\n",
            check.id.0,
            one_line(&check.text)
        ));
    }
    let mut takeovers = String::new();
    for event in input.timeline_events {
        match event {
            crate::timeline::TimelineEvent::Takeover {
                start_ms,
                end_ms,
                reason,
            } => {
                let at = input.time_map.video_time(*start_ms).0 / 1000;
                let length = end_ms.0.saturating_sub(start_ms.0) / 1000;
                let reason = reason
                    .as_deref()
                    .map(|r| format!(": {}", one_line(r)))
                    .unwrap_or_default();
                let period = if reason.ends_with('.') { "" } else { "." };
                takeovers.push_str(&format!("- At {}:{:02} in the video, the developer was in control for {}:{:02}{reason}{period}\n", at / 60, at % 60, length / 60, length % 60));
            }
            crate::timeline::TimelineEvent::Call { .. }
            | crate::timeline::TimelineEvent::CheckMark { .. } => {}
        }
    }
    if !takeovers.is_empty() {
        text.push_str("\n## Takeovers\n\n");
        text.push_str(&takeovers);
    }
    let profile = input.profile;
    // A recovered session from before Project Profiles has no start command.
    if !profile.start.is_empty() {
        let install = if profile.install.is_empty() {
            "none".to_owned()
        } else {
            code(&profile.install)
        };
        let toolchains = if profile.toolchains.is_empty() {
            "the sandbox's /usr/bin".to_owned()
        } else {
            profile
                .toolchains
                .iter()
                .map(|tool| format!("{} {}", tool.tool, tool.version))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let env = if profile.env.is_empty() {
            "none".to_owned()
        } else {
            profile.env.join(", ")
        };
        text.push_str(&format!(
            "\n## Project Profile\n\n- Install: {install}\n- Start: {}\n- Toolchains: {toolchains}\n- Environment: {env}\n",
            code(&profile.start)
        ));
    }
    if !input.summary.trim().is_empty() {
        text.push_str(&format!("\n## Verifier summary\n\n{}\n", input.summary));
    }
    match input.end {
        SessionEnd::Verdict(v) => text.push_str(&format!("\n## Verdict\n\n{v}\n")),
        SessionEnd::Incomplete(reason) => text.push_str(&format!(
            "\n## Incomplete Session\n\n{}\n",
            reason.sentence()
        )),
    }
    if let Some(error) = input.video_error {
        text.push_str(&format!(
            "\n## Verification Video\n\n{}\n\nThe Check and Takeover times above are positions in the Raw Recording.\n",
            one_line(error)
        ));
    }
    if let Some(error) = input.actions_error {
        text.push_str(&format!("\n## Action Log\n\n{}\n", one_line(error)));
    }
    text.push_str("\nTimes come from the Session Timeline, not from actions/.\n");
    text
}
pub fn index_line(
    date: &str,
    time: &str,
    revision: &Revision,
    end: &SessionEnd,
    goal: &str,
    links: &[(&str, String)],
) -> String {
    let goal = one_line(goal);
    let links = links
        .iter()
        .map(|(label, path)| format!("[{label}]({path})"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{date} {time} | {} | {} | {} | {goal} | {links}\n",
        revision.branch.as_deref().unwrap_or("detached HEAD"),
        revision.commit.get(..7).unwrap_or(&revision.commit),
        end.label()
    )
}
