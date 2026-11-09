use super::*;

#[test]
fn the_test_files_folder_sits_beside_the_project_profile_as_project_files() {
    let config = Path::new("/config/keek/projects");
    assert_eq!(folder(config, "app"), config.join("app.files"));
    assert_eq!(
        folder(config, "app").parent(),
        crate::project::profile_path(config, "app").parent()
    );
}

#[tokio::test]
async fn a_project_without_a_files_folder_has_no_test_files_and_no_folder() {
    let config = tempfile::tempdir().unwrap();
    let files = TestFiles::read(config.path(), "app").await.unwrap();
    assert!(files.folder.is_none());
    assert_eq!(files.list, TestFileList::default());
}

#[tokio::test]
async fn test_files_are_listed_by_relative_path_in_sorted_order_including_those_in_subfolders() {
    let config = tempfile::tempdir().unwrap();
    let root = config.path().join("app.files");
    std::fs::create_dir_all(root.join("docs/empty")).unwrap();
    for name in ["z.pdf", "docs/a.pdf", ".hidden"] {
        std::fs::write(root.join(name), "input").unwrap();
    }
    let files = TestFiles::read(config.path(), "app").await.unwrap();
    assert_eq!(files.folder, Some(root));
    assert_eq!(files.list.names, [".hidden", "docs/a.pdf", "z.pdf"]);
    assert_eq!(files.list.more, 0);
}

#[tokio::test]
async fn a_listing_past_100_names_keeps_100_and_counts_the_rest() {
    let config = tempfile::tempdir().unwrap();
    let root = config.path().join("app.files");
    std::fs::create_dir(&root).unwrap();
    for index in 0..103 {
        std::fs::write(root.join(format!("{index:03}")), "input").unwrap();
    }
    let files = TestFiles::read(config.path(), "app").await.unwrap();
    assert_eq!(
        files.list.names,
        (0..100).map(|i| format!("{i:03}")).collect::<Vec<_>>()
    );
    assert_eq!(files.list.more, 3);
}

#[tokio::test]
async fn a_symlink_in_the_files_folder_is_not_listed() {
    let config = tempfile::tempdir().unwrap();
    let root = config.path().join("app.files");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/a.pdf"), "input").unwrap();
    std::os::unix::fs::symlink("docs/a.pdf", root.join("link")).unwrap();
    std::os::unix::fs::symlink("docs", root.join("linked-folder")).unwrap();
    let files = TestFiles::read(config.path(), "app").await.unwrap();
    assert_eq!(files.list.names, ["docs/a.pdf"]);
}

#[tokio::test]
async fn a_files_path_that_is_a_regular_file_refuses_and_names_the_path() {
    let config = tempfile::tempdir().unwrap();
    let path = config.path().join("app.files");
    std::fs::write(&path, "input").unwrap();
    let error = TestFiles::read(config.path(), "app").await.unwrap_err();
    assert!(matches!(error, TestFilesError::NotAFolder(_)));
    let message = error.to_string();
    assert!(message.contains(path.to_str().unwrap()), "{message}");
    assert!(
        message.contains("Remove it or replace it with a folder"),
        "{message}"
    );
}

#[tokio::test]
async fn a_symlinked_test_files_folder_is_refused_with_an_explanation() {
    let config = tempfile::tempdir().unwrap();
    let target = config.path().join("actual");
    std::fs::create_dir(&target).unwrap();
    let path = config.path().join("app.files");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let error = TestFiles::read(config.path(), "app").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "{} is a symlink, and keek binds only a real folder as Test Files. Replace it with a folder holding copies of the files, then open again.",
            path.display()
        )
    );
}
