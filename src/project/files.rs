use super::ProjectError;
use crate::process::ProcessRunner;
use std::ffi::OsStr;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Component, Path, PathBuf};
pub struct ProjectFiles {
    entries: Vec<PathBuf>,
    pub(super) tracked: Vec<PathBuf>,
}
fn paths(bytes: &[u8]) -> Vec<PathBuf> {
    let mut paths: Vec<_> = bytes
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|b| PathBuf::from(OsStr::from_bytes(b)))
        .filter(|p| p.components().all(|c| matches!(c, Component::Normal(_))))
        .collect();
    paths.sort();
    paths.dedup();
    paths
}
impl ProjectFiles {
    pub async fn read(runner: &dyn ProcessRunner, worktree: &Path) -> Result<Self, ProjectError> {
        let args: Vec<_> = crate::revision::GIT_WITHOUT_REPOSITORY_PROGRAMS
            .into_iter()
            .chain(["ls-files", "-co", "--exclude-standard", "-z"])
            .collect();
        let entries = paths(&runner.output_bytes("git", &args, Some(worktree)).await?);
        let args: Vec<_> = crate::revision::GIT_WITHOUT_REPOSITORY_PROGRAMS
            .into_iter()
            .chain(["ls-files", "-z"])
            .collect();
        let tracked = paths(&runner.output_bytes("git", &args, Some(worktree)).await?);
        Ok(Self { entries, tracked })
    }
    pub fn build_tree(&self, worktree: &Path, tree: &Path) -> Result<(), ProjectError> {
        match std::fs::remove_dir_all(tree) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        std::fs::create_dir_all(tree)?;
        // Entries are sorted, so siblings usually share the parent checked last.
        let mut real_parent: Option<&Path> = None;
        for path in &self.entries {
            let parent = path.parent().unwrap_or(Path::new(""));
            if real_parent != Some(parent) {
                if !is_real_folder(worktree, parent)? {
                    continue;
                }
                real_parent = Some(parent);
            }
            let source = worktree.join(path);
            let target = tree.join(path);
            let metadata = match std::fs::symlink_metadata(&source) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if metadata.is_file() {
                link_file(&source, &target, &metadata)?;
            } else if metadata.is_symlink() {
                symlink(std::fs::read_link(&source)?, &target)?;
            } else if metadata.is_dir() {
                std::fs::create_dir_all(&target)?;
            }
        }
        Ok(())
    }
}
// The index can list a path below a symlink the listing itself leaves out, such as a git-ignored one, and reading it would copy a host file into the sandbox.
fn is_real_folder(worktree: &Path, relative: &Path) -> Result<bool, ProjectError> {
    let mut folder = worktree.to_owned();
    for component in relative {
        folder.push(component);
        match std::fs::symlink_metadata(&folder) {
            Ok(metadata) if metadata.is_dir() => (),
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}
fn link_file(source: &Path, target: &Path, metadata: &std::fs::Metadata) -> std::io::Result<()> {
    let input = std::fs::File::open(source)?;
    let output = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(target)?;
    // SAFETY: FICLONE takes a valid source descriptor and clones into the open destination.
    if unsafe { libc::ioctl(output.as_raw_fd(), libc::FICLONE, input.as_raw_fd()) } == 0 {
        output.set_permissions(metadata.permissions())?;
        output.set_times(std::fs::FileTimes::new().set_modified(metadata.modified()?))?;
        return Ok(());
    }
    drop(output);
    std::fs::copy(source, target)?;
    std::fs::File::open(target)?
        .set_times(std::fs::FileTimes::new().set_modified(metadata.modified()?))?;
    Ok(())
}
// git's top level honours core.worktree, so a repository's config could otherwise choose the host folder copied into the sandbox.
pub fn check_git_root(passed: &Path, root: &Path) -> Result<(), ProjectError> {
    let git_folder = passed
        .ancestors()
        .find(|folder| folder.join(".git").exists());
    if git_folder == Some(root) {
        Ok(())
    } else {
        Err(ProjectError::MovedWorktree {
            passed: passed.to_owned(),
            root: root.to_owned(),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::HostRunner;
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn git_top_level_with_core_worktree(repository: &Path, core_worktree: &Path) -> PathBuf {
        let git = |args: &[&std::ffi::OsStr]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(repository)
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap()
        };
        git(&[OsStr::new("init"), OsStr::new("-q")]);
        git(&[
            OsStr::new("config"),
            OsStr::new("core.worktree"),
            core_worktree.as_os_str(),
        ]);
        let top = git(&[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")]);
        std::fs::canonicalize(top.trim()).unwrap()
    }
    #[test]
    fn git_root_is_refused_when_core_worktree_names_another_folder() {
        let repository = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let other_path = std::fs::canonicalize(other.path()).unwrap();
        let passed = std::fs::canonicalize(repository.path()).unwrap();
        let root = git_top_level_with_core_worktree(&passed, &other_path);
        assert_eq!(root, other_path);
        let error = check_git_root(&passed, &root).unwrap_err().to_string();
        assert!(error.contains("core.worktree"), "{error}");
    }
    #[test]
    fn git_root_is_refused_when_core_worktree_names_an_ancestor() {
        let parent = tempfile::tempdir().unwrap();
        let parent_path = std::fs::canonicalize(parent.path()).unwrap();
        let passed = parent_path.join("repository");
        std::fs::create_dir(&passed).unwrap();
        // git resolves a relative core.worktree from the .git folder.
        let root = git_top_level_with_core_worktree(&passed, Path::new("../.."));
        assert_eq!(root, parent_path);
        assert!(check_git_root(&passed, &root).is_err());
    }
    #[test]
    fn git_root_is_accepted_from_a_subfolder_and_with_a_git_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(root.join(".git"), "gitdir: elsewhere\n").unwrap();
        check_git_root(&sub, &root).unwrap();
        check_git_root(&root, &root).unwrap();
    }
    fn repository() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(d.path())
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        };
        git(&["init", "-q"]);
        d
    }
    fn track(d: &Path, path: &str) {
        assert!(
            std::process::Command::new("git")
                .args(["add", path])
                .current_dir(d)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    async fn populated_tree() -> tempfile::TempDir {
        let d = repository();
        let r = d.path();
        for (p, c) in [
            ("tracked", "source"),
            ("untracked", "local"),
            (".gitignore", ".env\n"),
            (".env", "fake"),
            ("gone", "gone"),
            ("we ird\nname", "newline"),
        ] {
            std::fs::write(r.join(p), c).unwrap();
        }
        for p in ["tracked", ".gitignore", "gone", "we ird\nname"] {
            track(r, p);
        }
        symlink("tracked", r.join("link")).unwrap();
        track(r, "link");
        std::fs::remove_file(r.join("gone")).unwrap();
        std::fs::set_permissions(r.join("tracked"), std::fs::Permissions::from_mode(0o751))
            .unwrap();
        std::fs::create_dir(r.join("nested")).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(r.join("nested"))
                .output()
                .unwrap()
                .status
                .success()
        );
        std::fs::write(r.join("nested/hidden"), "hidden").unwrap();
        let f = ProjectFiles::read(&HostRunner, r).await.unwrap();
        let out = tempfile::tempdir().unwrap();
        f.build_tree(r, out.path()).unwrap();
        out
    }
    #[tokio::test]
    async fn project_tree_hides_ignored_and_deleted_files() {
        let out = populated_tree().await;
        for name in [".env", "gone", ".git"] {
            assert!(!out.path().join(name).exists());
        }
    }
    #[tokio::test]
    async fn project_tree_copies_tracked_and_untracked_contents() {
        let out = populated_tree().await;
        assert_eq!(
            std::fs::read_to_string(out.path().join("tracked")).unwrap(),
            "source"
        );
        assert_eq!(
            std::fs::read_to_string(out.path().join("untracked")).unwrap(),
            "local"
        );
        assert_eq!(
            std::fs::read_to_string(out.path().join("we ird\nname")).unwrap(),
            "newline"
        );
    }
    #[tokio::test]
    async fn project_tree_preserves_symlink_targets() {
        let out = populated_tree().await;
        assert_eq!(
            std::fs::read_link(out.path().join("link")).unwrap(),
            PathBuf::from("tracked")
        );
    }
    #[tokio::test]
    async fn project_tree_leaves_nested_repositories_empty() {
        let out = populated_tree().await;
        assert_eq!(
            std::fs::read_dir(out.path().join("nested"))
                .unwrap()
                .count(),
            0
        );
    }
    #[tokio::test]
    async fn project_tree_preserves_file_mode() {
        let out = populated_tree().await;
        assert_eq!(
            std::fs::metadata(out.path().join("tracked"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o751
        );
    }
    #[test]
    fn rebuilding_a_tree_does_not_truncate_a_hard_linked_source() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        let tree = root.path().join("tree");
        std::fs::create_dir(&work).unwrap();
        std::fs::create_dir(&tree).unwrap();
        std::fs::write(work.join("file"), "original").unwrap();
        std::fs::hard_link(work.join("file"), tree.join("file")).unwrap();
        let files = ProjectFiles {
            entries: paths(b"file\0"),
            tracked: vec![],
        };
        files.build_tree(&work, &tree).unwrap();
        assert_eq!(
            std::fs::read_to_string(work.join("file")).unwrap(),
            "original"
        );
    }
    #[test]
    fn tree_skips_descendants_of_files_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), "source").unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();
        std::fs::write(outside.path().join("child"), "outside").unwrap();
        let files = ProjectFiles {
            entries: paths(b"file\0file/child\0link\0link/child\0"),
            tracked: vec![],
        };
        files.build_tree(root.path(), tree.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(tree.path().join("file")).unwrap(),
            "source"
        );
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
        assert!(tree.path().join("link").is_symlink());
    }
    #[test]
    fn project_tree_does_not_read_through_a_symlink_missing_from_the_entries() {
        let root = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("child"), "made up").unwrap();
        std::fs::create_dir(root.path().join("folder")).unwrap();
        symlink(outside.path(), root.path().join("folder/link")).unwrap();
        let files = ProjectFiles {
            entries: paths(b"folder/link/child\0"),
            tracked: vec![],
        };
        files.build_tree(root.path(), tree.path()).unwrap();
        assert!(!tree.path().join("folder/link/child").exists());
    }
    #[test]
    fn project_tree_drops_entries_that_escape_the_worktree() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        let tree = root.path().join("output/tree");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(root.path().join("escape"), "escape").unwrap();
        std::fs::write(work.join("kept"), "kept").unwrap();
        let files = ProjectFiles {
            entries: paths(b"../escape\0kept\0"),
            tracked: vec![],
        };
        files.build_tree(&work, &tree).unwrap();
        assert!(tree.join("kept").exists());
        assert!(!root.path().join("output/escape").exists());
    }
}
