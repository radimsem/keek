use crate::checklist::{Check, Mark, SessionEnd};
use crate::session::Revision;
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
    pub checks: &'a [Check],
    pub end: &'a SessionEnd,
    pub time_map: &'a dyn VideoTimeMap,
}
// open_session takes the goal and Checks as free text, and a newline would break a heading,
// a list item or an index line.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
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
    if !input.summary.trim().is_empty() {
        text.push_str(&format!("\n## Verifier summary\n\n{}\n", input.summary));
    }
    match input.end {
        SessionEnd::Verdict(v) => text.push_str(&format!("\n## Verdict\n\n{v}\n")),
        SessionEnd::Incomplete { reason } => {
            text.push_str(&format!("\n## Incomplete Session\n\n{reason}\n"))
        }
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
