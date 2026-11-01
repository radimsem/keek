mod files;
mod layer;
mod settings;
pub use files::{ProjectFiles, ProjectKey, check_git_root};
pub use layer::{InstallFuture, Installer, ensure_layer};
pub use settings::{HostPort, Settings, WindowSize, read_project_settings};
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error(
        "Cannot prepare the Project: {0}. Check the worktree and cache permissions, then open again."
    )]
    Io(#[from] std::io::Error),
    #[error("Cannot list the Project files: {0}. Check the repository, then open again.")]
    Process(#[from] crate::process::ProcessError),
    #[error("Cannot prepare the Project: {0}. Check the worktree and open again.")]
    Task(#[from] tokio::task::JoinError),
    #[error(
        ".keek/setup.sh is missing or git-ignored. Commit a Setup Script with install and start phases, then open again."
    )]
    MissingScript,
    #[error(
        "git names {root} as the top of {passed}, but its .git is not there, so a core.worktree setting has probably moved it. Remove core.worktree from the repository's config, or pass the folder that holds .git, then open again."
    )]
    MovedWorktree { passed: PathBuf, root: PathBuf },
    #[error("Cannot install the Project: {reason}. Fix the Setup Script and open again. {log}")]
    Install { reason: String, log: String },
}
