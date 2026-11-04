use super::*;

#[tokio::test]
async fn open_session_without_a_project_profile_refuses_and_names_set_project_profile() {
    let mut h = Harness::new(2);
    let profile = h.registry.dependencies.projects_config.join("app.toml");
    let _ = std::fs::remove_file(&profile);
    let worktree = h.worktree("one");
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec!["check".into()],
        })
        .await;
    let message = refused(response);
    assert!(message.contains("set_project_profile"), "{message}");
    assert!(message.contains(profile.to_str().unwrap()), "{message}");
    assert!(h.registry.sessions.is_empty());
}
#[tokio::test]
async fn project_opened_warns_when_the_project_has_no_test_credentials() {
    let mut h = Harness::new(2);
    let worktree = h.worktree("one");
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec!["check".into()],
        })
        .await;
    let Response::Opened { warnings, .. } = response else {
        panic!("{response:?}");
    };
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("Project app") && warnings[0].contains("app.env"));
    assert!(warnings[0].contains("Project name is misspelled"));
}
#[tokio::test]
async fn project_tree_is_removed_when_a_session_finishes() {
    let mut h = Harness::new(2);
    let worktree = h.worktree("one");
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec!["check".into()],
        })
        .await;
    let Response::Opened { session, .. } = response else {
        panic!("{response:?}");
    };
    let tree = h.registry.dependencies.trees.join(session.to_string());
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("file"), "fixture").unwrap();
    h.finish(session).await;
    assert!(!tree.exists());
}
#[tokio::test]
async fn project_open_refuses_a_git_top_level_without_its_git_folder() {
    let mut h = Harness::new(2);
    let worktree = h.worktree("one");
    std::fs::remove_dir(worktree.join(".git")).unwrap();
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec![],
        })
        .await;
    assert!(refused(response).contains("core.worktree"));
    assert!(h.registry.sessions.is_empty());
}
#[tokio::test]
async fn project_invalid_settings_refuse_before_claiming_a_session() {
    let mut h = Harness::new(2);
    let config = h.registry.dependencies.projects_config.clone();
    std::fs::create_dir_all(&config).unwrap();
    let path = config.join("app.toml");
    std::fs::write(&path, "ports = [0]").unwrap();
    let worktree = h.worktree("one");
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec![],
        })
        .await;
    assert!(refused(response).contains(path.to_str().unwrap()));
    assert!(h.registry.sessions.is_empty());
    assert!(h.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn project_tree_is_removed_when_prepare_refuses_an_open() {
    struct Refuse(PathBuf);
    impl DriverFactory for Refuse {
        fn prepare<'a>(
            &'a self,
            folder: &'a SessionFolder,
            _: &'a crate::session::Opening,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<crate::session::Mounts, SessionError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                std::fs::create_dir_all(self.0.join(folder.root().file_name().unwrap()))?;
                Err(SessionError::Refused("fixture install refused".into()))
            })
        }
        fn open<'a>(
            &'a self,
            _: &'a SessionFolder,
            _: &'a crate::session::Opening,
            _: &'a crate::session::Mounts,
        ) -> crate::session::OpenDriverFuture<'a> {
            Box::pin(async { panic!("open called after refused prepare") })
        }
    }
    let mut h = Harness::new(2);
    let trees = h.registry.dependencies.trees.clone();
    h.registry.dependencies.factory = Arc::new(Refuse(trees.clone()));
    let worktree = h.worktree("one");
    let (reply, answer) = oneshot::channel();
    h.registry
        .handle(
            Request::OpenSession {
                project: "app".into(),
                worktree,
                goal: "goal".into(),
                checks: vec![],
            },
            reply,
            None,
        )
        .await;
    let prepared = h.registry.preparing.join_next().await.unwrap().unwrap();
    h.registry.prepared(prepared).await;
    let finished = h.registry.tasks.join_next().await.unwrap().unwrap();
    h.registry.finished(finished).await;
    assert!(refused(answer.await.unwrap()).contains("fixture install refused"));
    assert_eq!(std::fs::read_dir(trees).unwrap().count(), 0);
}

#[tokio::test]
async fn credentials_names_and_mode_warning_reach_opened_and_goal_is_masked() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::new(2);
    let config = h.registry.dependencies.projects_config.clone();
    std::fs::create_dir_all(&config).unwrap();
    let path = config.join("app.env");
    std::fs::write(&path, "PASSWORD=made-up-value").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let worktree = h.worktree("one");
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "made-up-value".into(),
            checks: vec!["made-up-value".into()],
        })
        .await;
    let Response::Opened {
        session,
        credentials,
        warnings,
        checks,
    } = response
    else {
        panic!("open refused")
    };
    assert_eq!(credentials, ["PASSWORD"]);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("644") && w.contains("chmod 600"))
    );
    assert_eq!(checks[0].text, "[masked]");
    let record = std::fs::read_to_string(
        h.registry
            .dependencies
            .sessions_dir
            .join(session.to_string())
            .join("session.json"),
    )
    .unwrap();
    assert!(!record.contains("made-up-value"));
    h.finish(session).await;
}
#[tokio::test]
async fn short_credentials_refuse_open_session() {
    let mut h = Harness::new(2);
    let config = h.registry.dependencies.projects_config.clone();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("app.env"), "PASSWORD=short").unwrap();
    let worktree = h.worktree("one");
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec![],
        })
        .await;
    assert!(refused(response).contains("longer"));
    assert!(h.calls.lock().unwrap().is_empty());
}
