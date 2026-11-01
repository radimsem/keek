use crate::checklist::{Mark, SessionEnd, Verdict};
use crate::report::{IdentityTimeMap, ReportInputs};
use crate::session::Actor;
use serde_json::{Value, json};
use std::path::PathBuf;

pub async fn file(
    actor: &Actor,
    verdict: Verdict,
    videos: PathBuf,
    summary: String,
) -> Result<Value, String> {
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
                time_map: &IdentityTimeMap,
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("The Verdict is {verdict}, but its Evidence was not filed. {e}"))?;
    Ok(json!({"verdict":verdict.to_string(),"checks":check_values,"evidence":filed}))
}
