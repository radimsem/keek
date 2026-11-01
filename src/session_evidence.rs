use crate::checklist::{Mark, SessionEnd, Verdict};
use crate::editor::{EditError, EditPlan};
use crate::report::{IdentityTimeMap, ReportInputs, VideoTimeMap};
use crate::session::Actor;
use serde_json::{Value, json};
use std::path::PathBuf;

pub async fn file(
    actor: &Actor,
    verdict: Verdict,
    videos: PathBuf,
    summary: String,
    edited: Result<EditPlan, EditError>,
) -> Result<Value, String> {
    let edit_error = edited.as_ref().err().map(ToString::to_string);
    let report_error = edit_error.clone();
    let folder = actor.folder.root().to_owned();
    let project = actor.project.clone();
    let session = actor.name.to_string();
    let started = actor.started;
    let goal = actor.goal.clone();
    let revision = actor.revision.clone();
    let checks = actor.checklist.checks().to_vec();
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
    let filed = tokio::task::spawn_blocking(move || {
        let identity = IdentityTimeMap;
        let time_map: &dyn VideoTimeMap = match &edited {
            Ok(plan) => plan,
            Err(_) => &identity,
        };
        crate::evidence::file(
            &folder,
            &videos,
            &project,
            &session,
            started,
            &ReportInputs {
                session: &session,
                goal: &goal,
                revision: &revision,
                summary: &summary,
                checks: &checks,
                end: &SessionEnd::Verdict(verdict),
                time_map,
                video_error: report_error.as_deref(),
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("The Verdict is {verdict}, but its Evidence was not filed. {e}"))?;
    let video_error = edit_error.map(|error| {
        let filed_recording = if filed.raw.is_some() {
            "The Raw Recording is filed with the other Evidence."
        } else {
            "No Raw Recording was available; the other Evidence is filed."
        };
        format!("{error} {filed_recording}")
    });
    let mut value = json!({"verdict":verdict.to_string(),"checks":check_values,"evidence":filed});
    if let Some(error) = video_error {
        tracing::error!(%error);
        value["video_error"] = json!(error);
    }
    Ok(value)
}
