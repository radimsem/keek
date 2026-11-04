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

fn write_profile(
    directory: &std::path::Path,
    project: &str,
    profile: serde_json::Value,
    credentials: Option<&CredentialSource>,
) -> Result<Response, String> {
    project.parse::<SessionName>().map_err(|e| e.to_string())?;
    let profile =
        crate::project::check_profile(&crate::project::profile_path(directory, project), profile)
            .map_err(|e| e.to_string())?;
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
