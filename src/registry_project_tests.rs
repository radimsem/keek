use super::*;

#[tokio::test]
async fn project_opened_carries_missing_settings_and_credentials_warnings() {
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
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings
            .iter()
            .all(|w| w.contains("Project app") && w.contains("Project name is misspelled"))
    );
    assert!(warnings.iter().any(|w| w.contains("app.toml")));
    assert!(warnings.iter().any(|w| w.contains("app.env")));
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
