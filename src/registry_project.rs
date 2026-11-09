use super::*;
use crate::ipc::CredentialSource;

impl Registry {
    pub(super) fn set_project_profile(
        &mut self,
        project: String,
        profile: serde_json::Value,
        credentials: Option<CredentialSource>,
        reply: oneshot::Sender<Response>,
    ) {
        let directory = self.dependencies.projects_config.clone();
        self.background.spawn(async move {
            let written = tokio::task::spawn_blocking(move || {
                write_profile(&directory, &project, profile, credentials.as_ref())
            })
            .await
            .unwrap_or_else(|error| {
                Err(format!(
                    "Cannot write the Project Profile: {error}. Try again."
                ))
            });
            let _ = reply.send(written.unwrap_or_else(|message| Response::Refused { message }));
        });
    }
}

// Two coding agents can set the same Project's profile at once. The credential copy is a
// read-modify-write through one temporary file, and each profile write moves the .bak.
static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn write_profile(
    directory: &std::path::Path,
    project: &str,
    profile: serde_json::Value,
    credentials: Option<&CredentialSource>,
) -> Result<Response, String> {
    project.parse::<SessionName>().map_err(|e| e.to_string())?;
    let profile =
        crate::project::check_profile(&crate::project::profile_path(directory, project), profile)
            .map_err(|e| e.to_string())?;
    let _writing = WRITES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let copied = match credentials {
        Some(credentials) => Some(
            crate::project::copy_credentials(
                &directory.join(format!("{project}.env")),
                &credentials.source,
                &credentials.names,
            )
            .map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let written =
        crate::project::write_profile(directory, project, &profile).map_err(|e| e.to_string())?;
    Ok(Response::ProfileWritten {
        path: written.path,
        profile: written.toml,
        credentials: copied,
    })
}

impl Registry {
    pub(super) fn prepare_project(
        &mut self,
        project: String,
        worktree: PathBuf,
        reply: oneshot::Sender<Response>,
        caller: opening::Caller,
    ) {
        let opening::Caller { mut opener, events } = caller;
        // A build whose task ended without a result has dropped its sender, so it is not joined.
        if let Some((running_worktree, running)) = self.prepares.get(&project)
            && running.borrow().is_none()
            && running.has_changed().is_ok()
        {
            if *running_worktree != worktree {
                let _ = reply.send(Response::Refused {
                    message: format!(
                        "Project {project} is being prepared from {}. Wait for that prepare_project to finish, then call it again for this worktree.",
                        running_worktree.display()
                    ),
                });
                return;
            }
            let mut running = running.clone();
            self.background.spawn(async move {
                let response = match running.wait_for(Option::is_some).await {
                    Ok(response) => response.clone().unwrap_or_else(ended),
                    Err(_) => ended(),
                };
                let _ = reply.send(response);
            });
            return;
        }
        let (done, running) = watch::channel(None);
        self.prepares
            .insert(project.clone(), (worktree.clone(), running));
        let runner = Arc::clone(&self.dependencies.runner);
        let factory = Arc::clone(&self.dependencies.factory);
        let sessions_dir = self.dependencies.sessions_dir.clone();
        let projects_config = self.dependencies.projects_config.clone();
        self.background.spawn(async move {
            let name = project.clone();
            let preparing = prepare(
                runner.as_ref(),
                factory.as_ref(),
                sessions_dir,
                projects_config,
                project,
                worktree,
                events,
            );
            let response = tokio::select! { biased;
                () = disconnected(&mut opener) => Response::Refused {
                    message: format!("The coding agent disconnected while Project {name} was being prepared. Call prepare_project again."),
                },
                response = preparing => response,
            };
            done.send_replace(Some(response.clone()));
            let _ = reply.send(response);
        });
    }
}

fn ended() -> Response {
    Response::Refused {
        message: "The Project's preparation ended without a result. Call prepare_project again."
            .into(),
    }
}

async fn disconnected(opener: &mut Option<watch::Receiver<()>>) {
    match opener {
        Some(receiver) => {
            let _ = receiver.changed().await;
        }
        None => std::future::pending().await,
    }
}

async fn prepare(
    runner: &dyn ProcessRunner,
    factory: &dyn crate::session::DriverFactory,
    sessions_dir: PathBuf,
    projects_config: PathBuf,
    project: String,
    worktree: PathBuf,
    events: Option<mpsc::Sender<Event>>,
) -> Response {
    let mut credentials = crate::credentials::Credentials::default();
    let result = async {
        project
            .parse::<SessionName>()
            .map_err(|e| SessionError::Refused(e.to_string()))?;
        let (profile, mut warnings) =
            crate::project::read_project_profile(&project, &projects_config).await?;
        let path = projects_config.join(format!("{project}.env"));
        let (read, warning) =
            tokio::task::spawn_blocking(move || crate::credentials::Credentials::read(&path))
                .await
                .map_err(|e| SessionError::Refused(e.to_string()))?
                .map_err(|e| SessionError::Refused(e.to_string()))?;
        credentials = read;
        warnings.extend(warning);
        let (worktree, _) = opening::checked_worktree(runner, sessions_dir, worktree).await?;
        let prep = crate::project_prepare::ProjectPrep {
            progress: crate::open_progress::OpenProgress::new(events, credentials.clone()),
            project,
            worktree,
            profile,
            credentials: credentials.clone(),
        };
        let mut prepared = factory.prepare_project(&prep).await?;
        prepared.warnings = warnings;
        Ok::<_, SessionError>(prepared)
    }
    .await;
    match result {
        Ok(prepared) => Response::Prepared(prepared),
        Err(error) => Response::Refused {
            message: credentials.mask(&error.to_string()),
        },
    }
}
