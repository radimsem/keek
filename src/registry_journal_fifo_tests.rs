use super::*;

#[tokio::test]
async fn recovery_removes_the_clipboard_fifo_from_a_retained_session_folder() {
    use std::os::unix::fs::FileTypeExt;
    let mut h = Harness::new(1);
    let env = h.registry.dependencies.projects_config.join("app.env");
    std::fs::create_dir_all(env.parent().unwrap()).unwrap();
    std::fs::write(&env, "PASSWORD=fictional-secret").unwrap();
    let name = h.open(h.worktree("fifo-recovery")).await;
    let folder = h.registry.dependencies.sessions_dir.join(name.to_string());
    crate::takeover_clipboard::create_fifo(&folder).unwrap();
    stop_without_filing(&mut h).await;
    // Changed Test Credentials withhold the Action Log, so the session folder stays.
    std::fs::write(&env, "PASSWORD=different-secret").unwrap();
    recover(&mut h).await;
    assert!(folder.exists(), "the session folder was not retained");
    let fifo = folder.join(crate::takeover_clipboard::FIFO_NAME);
    assert!(
        std::fs::symlink_metadata(&fifo).is_err(),
        "recovery left {}",
        fifo.display()
    );
    let evidence = h.directory.path().join("videos/keek/app/2026-01-15");
    for entry in std::fs::read_dir(&evidence).unwrap() {
        let path = entry.unwrap().path();
        assert!(
            !std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_fifo(),
            "the Evidence folder holds a FIFO: {}",
            path.display()
        );
    }
}
