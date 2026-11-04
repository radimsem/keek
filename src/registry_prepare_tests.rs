use super::*;
use crate::ipc::{LayerUse, PreparedProject, StartProbe};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

#[derive(Default)]
struct PrepareFactory {
    starts: AtomicUsize,
    finished: AtomicUsize,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl DriverFactory for PrepareFactory {
    fn open<'a>(
        &'a self,
        _: &'a SessionFolder,
        _: &'a crate::session::Opening,
        _: &'a crate::session::Mounts,
    ) -> crate::session::OpenDriverFuture<'a> {
        Box::pin(async { Err(SessionError::Refused("no desktop in this test".into())) })
    }
    fn prepare_project<'a>(
        &'a self,
        prep: &'a crate::project_prepare::ProjectPrep,
    ) -> crate::project_prepare::PrepareFuture<'a> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.release.notified().await;
            self.finished.fetch_add(1, Ordering::SeqCst);
            Ok(PreparedProject {
                project: prep.project.clone(),
                worktree: prep.worktree.clone(),
                toolchains: vec![],
                layer: LayerUse::Built,
                install_log: "installed\n".into(),
                start: StartProbe::Running,
                start_log: "listening\n".into(),
                warnings: vec![],
            })
        })
    }
}

fn harness() -> (Harness, Arc<PrepareFactory>, PathBuf) {
    let mut h = Harness::new(2);
    let factory = Arc::new(PrepareFactory::default());
    h.registry.dependencies.factory = factory.clone();
    let worktree = h.worktree("app");
    (h, factory, worktree)
}

async fn prepare(
    h: &mut Harness,
    worktree: &Path,
    opener: watch::Receiver<()>,
) -> oneshot::Receiver<Response> {
    let (reply, answer) = oneshot::channel();
    let request = Request::PrepareProject {
        project: "app".into(),
        worktree: worktree.to_owned(),
    };
    h.registry
        .handle_connection(Some(opener), request, reply, None)
        .await;
    answer
}

async fn within<T>(work: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .expect("timed out")
}

#[tokio::test]
async fn a_repeat_prepare_project_joins_the_build_a_cancelled_call_left_running() {
    let (mut h, factory, worktree) = harness();
    let (_first_client, opener) = watch::channel(());
    drop(prepare(&mut h, &worktree, opener).await);
    within(factory.started.notified()).await;
    let (_second_client, opener) = watch::channel(());
    let answer = prepare(&mut h, &worktree, opener).await;
    factory.release.notify_one();
    let response = within(answer).await.unwrap();
    let Response::Prepared(prepared) = response else {
        panic!("{response:?}");
    };
    assert_eq!(prepared.install_log, "installed\n");
    assert_eq!(prepared.start, StartProbe::Running);
    assert_eq!(factory.starts.load(Ordering::SeqCst), 1);
    assert_eq!(factory.finished.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_disconnect_of_the_starting_client_cancels_the_build_and_the_next_call_starts_again() {
    let (mut h, factory, worktree) = harness();
    let (first_client, opener) = watch::channel(());
    let answer = prepare(&mut h, &worktree, opener).await;
    within(factory.started.notified()).await;
    drop(first_client);
    let message = refused(within(answer).await.unwrap());
    assert!(message.contains("disconnected"), "{message}");
    assert_eq!(factory.finished.load(Ordering::SeqCst), 0);
    let (_second_client, opener) = watch::channel(());
    let answer = prepare(&mut h, &worktree, opener).await;
    within(factory.started.notified()).await;
    factory.release.notify_one();
    assert!(matches!(
        within(answer).await.unwrap(),
        Response::Prepared(_)
    ));
    assert_eq!(factory.starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn prepare_project_without_a_profile_refuses_and_names_set_project_profile() {
    let (mut h, factory, worktree) = harness();
    let config = h.registry.dependencies.projects_config.clone();
    std::fs::remove_file(config.join("app.toml")).unwrap();
    let (_client, opener) = watch::channel(());
    let message = refused(
        within(prepare(&mut h, &worktree, opener).await)
            .await
            .unwrap(),
    );
    assert!(message.contains("set_project_profile"), "{message}");
    assert_eq!(factory.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_prepare_project_for_another_worktree_while_one_runs_is_refused_with_the_running_worktree()
 {
    let (mut h, factory, worktree) = harness();
    let (_first_client, opener) = watch::channel(());
    let first = prepare(&mut h, &worktree, opener).await;
    within(factory.started.notified()).await;
    let other = h.worktree("other");
    let (_second_client, opener) = watch::channel(());
    let message = refused(within(prepare(&mut h, &other, opener).await).await.unwrap());
    assert!(
        message.contains(&worktree.display().to_string()),
        "{message}"
    );
    factory.release.notify_one();
    assert!(matches!(
        within(first).await.unwrap(),
        Response::Prepared(_)
    ));
    assert_eq!(factory.starts.load(Ordering::SeqCst), 1);
}
