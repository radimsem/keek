mod credential_copy;
mod files;
mod key;
pub(crate) mod layer;
mod profile;
mod profile_file;
pub use credential_copy::{CopiedCredentials, CopyError, copy_credentials};
pub use files::{ProjectFiles, check_git_root};
pub use key::ProjectKey;
pub use layer::{InstallFuture, Installer, ensure_layer};
pub use profile::{
    HostPort, Profile, ProfileError, WindowSize, profile_path, read_project_profile,
};
pub use profile_file::{WrittenProfile, check_profile, write_profile};
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
        "git names {root} as the top of {passed}, but its .git is not there, so a core.worktree setting has probably moved it. Remove core.worktree from the repository's config, or pass the folder that holds .git, then open again."
    )]
    MovedWorktree { passed: PathBuf, root: PathBuf },
    #[error(
        "Cannot install the Project: {reason}. Fix the install command with set_project_profile and try again. {log}"
    )]
    Install { reason: String, log: String },
}
