use super::*;

#[tokio::test]
async fn recovery_removes_every_worktree_copy_and_still_files_the_incomplete_session() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("leftover-trees")).await;
    stop_without_filing(&mut h).await;
    let trees = h.registry.dependencies.trees.clone();
    let session_tree = trees.join(name.to_string());
    std::fs::create_dir_all(&session_tree).unwrap();
    std::fs::write(session_tree.join("f"), "session").unwrap();
    let orphan = trees.join("orphan/sub");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("f"), "orphan").unwrap();
    std::fs::set_permissions(orphan, std::fs::Permissions::from_mode(0o500)).unwrap();
    std::os::unix::fs::symlink(trees.join("missing"), trees.join("dangling")).unwrap();
    std::fs::write(trees.join("file"), "leftover").unwrap();
    recover(&mut h).await;
    assert!(std::fs::read_dir(trees).unwrap().next().is_none());
    assert!(
        h.directory
            .path()
            .join(format!(
                "videos/keek/app/2026-01-15/{name}_incomplete_report.md"
            ))
            .exists()
    );
}

#[tokio::test]
async fn recovery_replaces_planted_output_directories() {
    let mut h = Harness::new(1);
    let name = h.open(h.worktree("planted-directories")).await;
    stop_without_filing(&mut h).await;
    let folder = h.registry.dependencies.sessions_dir.join(name.to_string());
    for entry in ["verification.mp4", "recovery-timeline.tmp", "timeline.json"] {
        let path = folder.join(entry);
        if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
        let nested = path.join("x/y");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("z"), "planted").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(nested, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    recover(&mut h).await;
    assert!(
        h.directory
            .path()
            .join(format!(
                "videos/keek/app/2026-01-15/{name}_incomplete_report.md"
            ))
            .exists()
    );
}

#[tokio::test]
async fn an_unwritable_journals_directory_refuses_open_without_a_session_folder() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::new(1);
    let parent = h.directory.path().join("locked");
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    h.registry.dependencies.journals_dir = parent.join("journals");
    let worktree = h.worktree("journal-failure");
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec!["check".into()],
        })
        .await;
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(response, Response::Refused { .. }), "{response:?}");
    let sessions = &h.registry.dependencies.sessions_dir;
    assert!(!sessions.exists() || std::fs::read_dir(sessions).unwrap().next().is_none());
}
