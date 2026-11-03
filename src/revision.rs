use crate::session::SessionError;
use std::path::Path;
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Revision {
    pub branch: Option<String>,
    pub commit: String,
    pub dirty: bool,
}
// git runs on the host, in a folder the caller picks, and a repository's config can
// name programs. `filter.<name>.clean` can't be turned off this way.
pub(crate) const GIT_WITHOUT_REPOSITORY_PROGRAMS: [&str; 5] = [
    "--no-lazy-fetch",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.hooksPath=/dev/null",
];
async fn git(
    runner: &dyn crate::process::ProcessRunner,
    path: &Path,
    args: &[&str],
) -> Result<String, crate::process::ProcessError> {
    let arguments: Vec<&str> = GIT_WITHOUT_REPOSITORY_PROGRAMS
        .iter()
        .chain(args)
        .copied()
        .collect();
    runner.output("git", &arguments, Some(path)).await
}
pub async fn revision(
    runner: &dyn crate::process::ProcessRunner,
    path: &Path,
) -> Result<(std::path::PathBuf, Revision), SessionError> {
    let error = |e: crate::process::ProcessError| {
        SessionError::Refused(format!(
            "Cannot read git worktree {}: {e}. Pass an existing git worktree.",
            path.display()
        ))
    };
    let root = git(runner, path, &["rev-parse", "--show-toplevel"])
        .await
        .map_err(error)?;
    let branch = git(
        runner,
        path,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )
    .await
    .ok();
    let commit = git(runner, path, &["rev-parse", "HEAD"])
        .await
        .map_err(error)?;
    let dirty = !git(runner, path, &["status", "--porcelain"])
        .await
        .map_err(error)?
        .is_empty();
    Ok((
        root.into(),
        Revision {
            branch,
            commit,
            dirty,
        },
    ))
}
