use super::*;

enum Failure {
    Prepare,
    Setup,
}
struct FailingFactory(Failure);
impl DriverFactory for FailingFactory {
    fn prepare<'a>(
        &'a self,
        _: &'a SessionFolder,
        _: &'a crate::session::Opening,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::session::Mounts, SessionError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            match self.0 {
                Failure::Prepare => Err(SessionError::Refused("fixture prepare failed".into())),
                Failure::Setup => Ok(crate::session::Mounts::default()),
            }
        })
    }
    fn open<'a>(
        &'a self,
        _: &'a SessionFolder,
        _: &'a crate::session::Opening,
        _: &'a crate::session::Mounts,
    ) -> crate::session::OpenDriverFuture<'a> {
        Box::pin(async { Ok(Box::new(FailingSetup) as Box<dyn Driver>) })
    }
}
struct FailingSetup;
impl Driver for FailingSetup {
    fn list_tools(&mut self) -> DriverFuture<'_, Vec<Tool>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn call_tool<'a>(&'a mut self, _: &'a str, _: Value) -> DriverFuture<'a, Value> {
        Box::pin(async {
            Err(crate::driver::DriverError::Call {
                tool: "setup".into(),
                reason: "fixture setup failed".into(),
            })
        })
    }
    fn close(&mut self) -> DriverFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}
async fn assert_failed_setup_is_removed(failure: Failure, reason: &str) {
    let mut h = Harness::new(1);
    h.registry.dependencies.factory = Arc::new(FailingFactory(failure));
    let worktree = h.worktree("failed");
    let message = refused(
        h.request(Request::OpenSession {
            project: "app".into(),
            worktree,
            goal: "goal".into(),
            checks: vec!["check".into()],
        })
        .await,
    );
    assert!(message.contains(reason), "{message}");
    assert!(!h.registry.keeps_daemon_alive());
    for folder in [
        &h.registry.dependencies.sessions_dir,
        &h.registry.dependencies.journals_dir,
    ] {
        assert_eq!(
            std::fs::read_dir(folder).unwrap().count(),
            0,
            "{} remains",
            folder.display()
        );
    }
}
#[tokio::test]
async fn a_failed_prepare_removes_the_session_folder_and_journal() {
    assert_failed_setup_is_removed(Failure::Prepare, "fixture prepare failed").await;
}
#[tokio::test]
async fn a_failed_driver_setup_removes_the_session_folder_and_journal() {
    assert_failed_setup_is_removed(Failure::Setup, "fixture setup failed").await;
}
