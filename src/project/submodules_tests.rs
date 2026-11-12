use crate::process::HostRunner;
use crate::project::ProjectFiles;
use std::path::Path;

fn git(directory: &Path, args: &[&str]) {
    let mut command = std::process::Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.as_encoded_bytes().starts_with(b"GIT_") {
            command.env_remove(name);
        }
    }
    let output = command
        .args(args)
        .current_dir(directory)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(in crate::project) fn repository_with_submodule() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let lib = directory.path().join("lib");
    let root = directory.path().join("super");
    std::fs::create_dir(&lib).unwrap();
    std::fs::create_dir(&root).unwrap();
    git(&lib, &["init", "-q"]);
    for (path, contents) in [
        ("tracked", "source"),
        (".gitignore", "*.env\n"),
        ("Cargo.lock", "first"),
    ] {
        std::fs::write(lib.join(path), contents).unwrap();
    }
    git(&lib, &["add", "."]);
    git(&lib, &["commit", "-qm", "source"]);
    git(&root, &["init", "-q"]);
    std::fs::write(root.join("top"), "superproject").unwrap();
    git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../lib",
            "vendor/lib",
        ],
    );
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "superproject"]);
    std::fs::write(root.join("vendor/lib/ignored.env"), "fake").unwrap();
    directory
}

async fn build_tree(root: &Path) -> tempfile::TempDir {
    let files = ProjectFiles::read(&HostRunner, root).await.unwrap();
    let tree = tempfile::tempdir().unwrap();
    files.build_tree(root, tree.path()).unwrap();
    tree
}

#[tokio::test]
async fn project_tree_copies_a_file_tracked_in_an_initialized_submodule() {
    let directory = repository_with_submodule();
    let tree = build_tree(&directory.path().join("super")).await;
    assert_eq!(
        std::fs::read_to_string(tree.path().join("vendor/lib/tracked")).unwrap(),
        "source"
    );
}

#[tokio::test]
async fn project_tree_hides_a_git_ignored_file_inside_a_submodule() {
    let directory = repository_with_submodule();
    let tree = build_tree(&directory.path().join("super")).await;
    assert!(tree.path().join("vendor/lib/tracked").exists());
    assert!(!tree.path().join("vendor/lib/ignored.env").exists());
}

#[tokio::test]
async fn project_files_refuse_a_submodule_whose_core_worktree_names_another_folder() {
    let directory = repository_with_submodule();
    let root = directory.path().join("super");
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("hostfile"), "host").unwrap();
    git(
        &root,
        &[
            "config",
            "--file",
            ".git/modules/vendor/lib/config",
            "core.worktree",
            other.path().to_str().unwrap(),
        ],
    );
    let error = ProjectFiles::read(&HostRunner, &root)
        .await
        .err()
        .expect("a moved submodule must be refused")
        .to_string();
    assert!(error.contains("core.worktree"), "{error}");
    assert!(
        error.contains(
            root.canonicalize()
                .unwrap()
                .join("vendor/lib")
                .to_str()
                .unwrap()
        ),
        "{error}"
    );
}

#[tokio::test]
async fn project_tree_leaves_an_uninitialized_submodule_empty() {
    let directory = repository_with_submodule();
    let root = directory.path().join("super");
    git(&root, &["submodule", "deinit", "-f", "--", "vendor/lib"]);
    let tree = build_tree(&root).await;
    let submodule = tree.path().join("vendor/lib");
    assert!(submodule.is_dir());
    assert_eq!(std::fs::read_dir(&submodule).unwrap().count(), 0);
}

#[tokio::test]
async fn project_tree_leaves_a_submodule_with_a_missing_git_directory_empty() {
    let directory = repository_with_submodule();
    let root = directory.path().join("super");
    std::fs::rename(
        root.join(".git/modules/vendor/lib"),
        root.join(".git/modules/vendor/missing"),
    )
    .unwrap();
    let tree = build_tree(&root).await;
    assert_eq!(
        std::fs::read_dir(tree.path().join("vendor/lib"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        std::fs::read_to_string(tree.path().join("top")).unwrap(),
        "superproject"
    );
}

#[tokio::test]
async fn project_tree_copies_a_submodule_whose_folder_name_ends_in_a_space() {
    let directory = repository_with_submodule();
    let root = directory.path().join("super");
    git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../lib",
            "vendor/lib ",
        ],
    );
    let tree = build_tree(&root).await;
    assert_eq!(
        std::fs::read_to_string(tree.path().join("vendor/lib /tracked")).unwrap(),
        "source"
    );
}

struct RecordingRunner(std::sync::Mutex<Vec<std::path::PathBuf>>);

impl crate::process::ProcessRunner for RecordingRunner {
    fn output_bytes<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> crate::process::BytesFuture<'a> {
        if program == "git"
            && let Some(folder) = cwd
        {
            self.0.lock().unwrap().push(folder.to_owned());
        }
        crate::process::ProcessRunner::output_bytes(&HostRunner, program, args, cwd)
    }

    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        if program == "git"
            && let Some(folder) = cwd
        {
            self.0.lock().unwrap().push(folder.to_owned());
        }
        crate::process::ProcessRunner::output(&HostRunner, program, args, cwd)
    }
}

#[tokio::test]
async fn project_files_never_run_git_inside_an_uninitialized_submodule() {
    let directory = repository_with_submodule();
    let root = directory.path().join("super");
    git(&root, &["submodule", "deinit", "-f", "--", "vendor/lib"]);
    let runner = RecordingRunner(std::sync::Mutex::new(Vec::new()));
    ProjectFiles::read(&runner, &root).await.unwrap();
    assert!(!runner.0.lock().unwrap().contains(&root.join("vendor/lib")));
}

#[tokio::test]
async fn project_tree_copies_an_untracked_file_that_a_submodule_does_not_ignore() {
    let directory = repository_with_submodule();
    let root = directory.path().join("super");
    std::fs::write(root.join(".gitignore"), "vendor/lib/untracked\n").unwrap();
    std::fs::write(root.join("vendor/lib/untracked"), "untracked").unwrap();
    let tree = build_tree(&root).await;
    assert_eq!(
        std::fs::read_to_string(tree.path().join("vendor/lib/untracked")).unwrap(),
        "untracked"
    );
}

#[tokio::test]
async fn project_tree_copies_files_from_a_submodule_inside_a_submodule() {
    let directory = repository_with_submodule();
    let root = directory.path().join("super");
    let nested = directory.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    git(&nested, &["init", "-q"]);
    std::fs::write(nested.join("tracked"), "nested").unwrap();
    git(&nested, &["add", "."]);
    git(&nested, &["commit", "-qm", "nested"]);
    git(
        &root.join("vendor/lib"),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            nested.to_str().unwrap(),
            "inner",
        ],
    );
    let tree = build_tree(&root).await;
    assert_eq!(
        std::fs::read_to_string(tree.path().join("vendor/lib/inner/tracked")).unwrap(),
        "nested"
    );
}
