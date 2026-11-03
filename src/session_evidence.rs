use crate::checklist::{Mark, SessionEnd};
use crate::editor::{EditError, EditPlan};
use crate::report::{IdentityTimeMap, ReportInputs, VideoTimeMap};
use crate::session::Actor;
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct Inputs {
    pub folder: PathBuf,
    pub project: String,
    pub name: String,
    pub started: chrono::DateTime<chrono::Local>,
    pub goal: String,
    pub revision: crate::revision::Revision,
    pub checks: Vec<crate::checklist::Check>,
    pub credentials: crate::credentials::Credentials,
    pub action_log: crate::evidence::ActionLog,
    pub actions_error: Option<String>,
}
impl From<&Actor> for Inputs {
    fn from(actor: &Actor) -> Self {
        Self {
            folder: actor.folder.root().to_owned(),
            project: actor.project.clone(),
            name: actor.name.to_string(),
            started: actor.started,
            goal: actor.goal.clone(),
            revision: actor.revision.clone(),
            checks: actor.checklist.checks().to_vec(),
            credentials: actor.credentials.clone(),
            action_log: crate::evidence::ActionLog::File,
            actions_error: None,
        }
    }
}
pub async fn file(
    input: Inputs,
    end: SessionEnd,
    videos: Result<PathBuf, String>,
    summary: String,
    edited: Result<EditPlan, EditError>,
    timeline: &crate::timeline::SessionTimeline,
) -> Result<Value, String> {
    let (failure, mut value) = match &end {
        SessionEnd::Verdict(verdict) => (
            format!("The Verdict is {verdict}, but its Evidence was not filed."),
            json!({"verdict":verdict.to_string()}),
        ),
        SessionEnd::Incomplete(reason) => (
            "The Incomplete Session's Evidence was not filed.".into(),
            json!({"incomplete":reason.sentence()}),
        ),
    };
    let edit_error = edited.as_ref().err().map(ToString::to_string);
    let report_error = edit_error.clone();
    let Inputs {
        folder,
        project,
        name: session,
        started,
        goal,
        revision,
        checks,
        credentials,
        action_log,
        actions_error,
    } = input;
    let check_values: Vec<_> = checks
        .iter()
        .map(|c| {
            let (outcome, comment) = match &c.mark {
                Mark::Unmarked => (None, None),
                Mark::Outcome {
                    outcome, comment, ..
                } => (Some(outcome), Some(comment)),
            };
            json!({"id":c.id,"text":c.text,"added":c.added,"outcome":outcome,"comment":comment})
        })
        .collect();
    let timeline_events = timeline.events().to_vec();
    let (filed, actions_error) = tokio::task::spawn_blocking(move || {
        let identity = IdentityTimeMap;
        let time_map: &dyn VideoTimeMap = match &edited {
            Ok(plan) => plan,
            Err(_) => &identity,
        };
        let actions_error = crate::credential_scrub::scrub_folder(&credentials, &folder)
            .err()
            .map(|error| {
                format!(
                    "The Action Log was not filed because Test Credentials could not be removed. {error} The Action Log stays in the session folder at {}. Check and delete it.",
                    folder.display()
                )
            }).or(actions_error);
        let action_log = if actions_error.is_some() {
            crate::evidence::ActionLog::Withhold
        } else {
            action_log
        };
        let videos = videos?;
        crate::evidence::file(
            &folder,
            &videos,
            &project,
            &session,
            started,
            action_log,
            &ReportInputs {
                timeline_events: &timeline_events,
                session: &session,
                goal: &goal,
                revision: &revision,
                summary: &summary,
                checks: &checks,
                end: &end,
                time_map,
                video_error: report_error.as_deref(),
                actions_error: actions_error.as_deref(),
            },
        ).map(|filed| (filed, actions_error)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{failure} {e}"))?;
    let video_error = edit_error.map(|error| {
        let filed_recording = if filed.raw.is_some() {
            "The Raw Recording is filed with the other Evidence."
        } else {
            "No Raw Recording was available; the other Evidence is filed."
        };
        format!("{error} {filed_recording}")
    });
    value["checks"] = json!(check_values);
    value["evidence"] = json!(filed);
    if let Some(error) = video_error {
        tracing::error!(%error);
        value["video_error"] = json!(error);
    }
    if let Some(error) = actions_error {
        value["actions_error"] = json!(error);
    }
    Ok(value)
}
