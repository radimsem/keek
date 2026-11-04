use super::*;
impl Registry {
    pub async fn recover(&mut self) {
        let directory = self.dependencies.journals_dir.clone();
        let journals = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<PathBuf>> {
            match std::fs::read_dir(directory) {
                Ok(entries) => entries
                    .filter_map(|entry| match entry {
                        Ok(entry) if entry.path().extension().is_some_and(|ext| ext == "json") => {
                            Some(Ok(entry.path()))
                        }
                        Ok(_) => None,
                        Err(error) => Some(Err(error)),
                    })
                    .collect(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
                Err(error) => Err(error),
            }
        })
        .await;
        match journals {
            Ok(Ok(journals)) => {
                for path in journals {
                    let sessions = self.dependencies.sessions_dir.clone();
                    let projects = self.dependencies.projects_config.clone();
                    let trees = self.dependencies.trees.clone();
                    let runner = self.dependencies.runner.clone();
                    self.recovery.spawn(async move {
                        if let Err(error) =
                            recover_one(&path, &sessions, &projects, &trees, runner.as_ref()).await
                        {
                            tracing::error!(%error, ?path, "could not recover session");
                        }
                    });
                }
            }
            result => tracing::error!(?result, "could not list session journals"),
        }
    }
}
async fn recover_one(
    path: &std::path::Path,
    sessions: &std::path::Path,
    projects: &std::path::Path,
    trees: &std::path::Path,
    runner: &dyn ProcessRunner,
) -> Result<(), SessionError> {
    // The sandbox can write session-folder files, so recovery trusts the journal.
    let source = path.to_owned();
    let journal: crate::session_journal::Journal =
        tokio::task::spawn_blocking(move || -> Result<_, SessionError> {
            Ok(serde_json::from_slice(&std::fs::read(source)?)?)
        })
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))??;
    journal
        .project
        .parse::<SessionName>()
        .map_err(|error| SessionError::Refused(error.to_string()))?;
    let folder = SessionFolder::new(sessions, &journal.name);
    let root = folder.root().to_owned();
    let duration = crate::editor::recording_duration(runner, &folder.recording()).await;
    let timeline = journal.recovery_timeline(
        duration
            .as_ref()
            .copied()
            .unwrap_or(crate::timeline::Millis(0)),
    );
    let bytes = serde_json::to_vec_pretty(&timeline)?;
    let exists = tokio::task::spawn_blocking(move || -> std::io::Result<bool> {
        if !root.try_exists()? {
            return Ok(false);
        }
        for path in [
            root.join("verification.mp4"),
            root.join("verification.mp4.parts"),
            root.join("recovery-timeline.tmp"),
            root.join("timeline.json"),
        ] {
            let result = match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() => crate::project::layer::remove_build(&path),
                Ok(_) => std::fs::remove_file(&path),
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(?path, %error, "could not remove stale recovery output")
                }
            }
        }
        let temporary = root.join("recovery-timeline.tmp");
        crate::session_folder::write_new(&temporary, &bytes)?;
        std::fs::rename(temporary, root.join("timeline.json"))?;
        Ok(true)
    })
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))??;
    if exists {
        let videos = crate::evidence::writable_videos_dir(runner)
            .await
            .map_err(|e| SessionError::Refused(e.to_string()))?;
        let env = projects.join(format!("{}.env", journal.project));
        let read = tokio::task::spawn_blocking(move || crate::credentials::Credentials::read(&env))
            .await
            .map_err(|e| SessionError::Refused(e.to_string()))?;
        let (credentials, action_log, actions_error) = match read {
            Ok((credentials, _)) if credentials.fingerprint() == journal.credentials_hash => {
                (credentials, crate::evidence::ActionLog::File, None)
            }
            Ok(_) | Err(_) => (
                Default::default(),
                crate::evidence::ActionLog::Withhold,
                Some(format!(
                    "The Project's Test Credentials file changed or was removed after the session, so keek withheld the Action Log. The Action Log stays in the session folder at {}. Check and delete it.",
                    folder.root().display()
                )),
            ),
        };
        let reason = crate::checklist::IncompleteReason::DaemonStopped;
        let edited = match duration {
            Ok(_) => {
                crate::editor::edit_incomplete(
                    runner,
                    &folder.recording(),
                    &timeline,
                    &folder.root().join("verification.mp4"),
                    &mut |_| {},
                    &reason,
                )
                .await
            }
            Err(error) => Err(error),
        };
        let started = chrono::DateTime::parse_from_rfc3339(&journal.started)
            .map_err(|e| SessionError::Refused(e.to_string()))?
            .with_timezone(&chrono::Local);
        let result = crate::session_evidence::file(
            crate::session_evidence::Inputs {
                folder: folder.root().to_owned(),
                project: journal.project,
                name: journal.name.to_string(),
                started,
                goal: journal.goal,
                revision: journal.revision,
                profile: journal.profile,
                checks: journal.checks,
                credentials,
                action_log,
                actions_error,
            },
            crate::checklist::SessionEnd::Incomplete(reason),
            Ok(videos),
            String::new(),
            edited,
            &timeline,
        )
        .await;
        let result = result.map_err(SessionError::Refused)?;
        if let Some(error) = result["actions_error"].as_str() {
            tracing::error!(%error, "recovered Action Log could not be filed");
        }
    } else {
        tracing::warn!(session = %journal.name, "session folder is gone; removing its journal");
    }
    let tree = crate::session_project::tree_path(trees, &journal.name);
    let cleanup = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(tree)).await;
    match cleanup {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        error => tracing::warn!(?error, "could not remove recovered worktree copy"),
    }
    crate::session_journal::remove(path.to_owned()).await;
    Ok(())
}
