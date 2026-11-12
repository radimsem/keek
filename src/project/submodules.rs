use super::ProjectError;
use super::files::{holds_git_root, is_real_folder, relative};
use crate::process::ProcessRunner;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(super) struct Staged {
    pub tracked: Vec<PathBuf>,
    pub gitlinks: Vec<PathBuf>,
}

// ls-files -s exposes mode 160000 gitlinks.
pub(super) fn staged(bytes: &[u8]) -> Staged {
    let mut staged = Staged {
        tracked: Vec::new(),
        gitlinks: Vec::new(),
    };
    for record in bytes.split(|byte| *byte == 0) {
        let mut parts = record.splitn(2, |byte| *byte == b'\t');
        let Some(header) = parts.next() else {
            continue;
        };
        let Some(path) = parts.next().and_then(relative) else {
            continue;
        };
        if header.split(|byte| *byte == b' ').next() == Some(b"160000") {
            staged.gitlinks.push(path.clone());
        }
        staged.tracked.push(path);
    }
    staged.gitlinks.sort();
    staged.gitlinks.dedup();
    staged
}

pub(super) async fn initialized(
    runner: &dyn ProcessRunner,
    worktree: &Path,
    path: &Path,
) -> Result<bool, ProjectError> {
    let worktree = worktree.to_owned();
    let path = path.to_owned();
    let folder = tokio::task::spawn_blocking(move || -> Result<_, ProjectError> {
        if !is_real_folder(&worktree, &path)? {
            return Ok(None);
        }
        let folder = worktree.join(path);
        // A submodule can hold a gitfile or a .git directory.
        match std::fs::symlink_metadata(folder.join(".git")) {
            Ok(metadata) if metadata.is_file() || metadata.is_dir() => Ok(Some(folder)),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    })
    .await??;
    let Some(folder) = folder else {
        return Ok(false);
    };
    let args: Vec<_> = crate::revision::GIT_WITHOUT_REPOSITORY_PROGRAMS
        .into_iter()
        .chain(["rev-parse", "--show-toplevel"])
        .collect();
    let root = match runner.output_bytes("git", &args, Some(&folder)).await {
        Ok(root) => root,
        Err(error) => {
            tracing::warn!(submodule = %folder.display(), %error, "could not inspect submodule");
            return Ok(false);
        }
    };
    let root = PathBuf::from(OsStr::from_bytes(root.strip_suffix(b"\n").unwrap_or(&root)));
    tokio::task::spawn_blocking(move || {
        // git reports a real path, and the folder may sit below a symlink.
        let submodule = std::fs::canonicalize(folder)?;
        let root = std::fs::canonicalize(root)?;
        if holds_git_root(&submodule, &root) {
            Ok(true)
        } else {
            Err(ProjectError::MovedSubmodule { submodule, root })
        }
    })
    .await?
}

#[cfg(test)]
#[path = "submodules_tests.rs"]
pub(super) mod tests;
