use super::*;
fn window(address: &str, pid: u32, class: &str) -> HostWindow {
    HostWindow {
        address: WindowAddress(address.into()),
        pid,
        class: class.into(),
    }
}
#[test]
fn the_session_window_and_other_pids_windows_are_not_stray() {
    let windows = [
        window("0xabc", 7, "aquamarine"),
        window("0xdef", 8, "foreign"),
        window("0xdef", 8, "aquamarine"),
    ];
    assert_eq!(
        stray_window(&windows, WindowPid(7), &WindowAddress("0xabc".into())),
        None
    );
}
#[test]
fn a_foreign_class_from_the_session_pid_is_stray() {
    let windows = [window("0xabc", 7, "foreign")];
    assert_eq!(
        stray_window(&windows, WindowPid(7), &WindowAddress("0xabc".into())),
        Some(&windows[0])
    );
}
#[test]
fn a_second_aquamarine_window_from_the_session_pid_is_stray() {
    let windows = [window("0xdef", 7, "aquamarine")];
    assert_eq!(
        stray_window(&windows, WindowPid(7), &WindowAddress("0xabc".into())),
        Some(&windows[0])
    );
}
use crate::compositor::*;
use crate::process::{ProcessError, ProcessRunner};
use std::sync::Mutex;
use tokio::sync::mpsc;
struct Host {
    snapshots: Mutex<std::collections::VecDeque<Result<Vec<HostWindow>, ProcessError>>>,
    calls: std::sync::atomic::AtomicUsize,
}
impl Compositor for Host {
    fn windows(&self) -> WindowsFuture<'_> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let result = self
            .snapshots
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(vec![]));
        Box::pin(async { result })
    }
    fn place_window(
        &self,
        _: Option<WindowPid>,
        _: Placement,
        _: Option<WindowEvents>,
    ) -> WindowFuture<'_> {
        unreachable!()
    }
    fn slot(
        &self,
        _: crate::viewer::SlotId,
        _: usize,
        _: Option<crate::project::WindowSize>,
    ) -> SlotFuture<'_> {
        unreachable!()
    }
    fn window_events(&self) -> EventsFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn install(&self) -> CompositorFuture<'_> {
        Box::pin(async { Ok(()) })
    }
    fn remove(&self) -> CompositorFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}
struct Events(mpsc::Receiver<Result<HyprEvent, ProcessError>>);
impl TakeoverEvents for Events {
    fn next(&mut self) -> EventFuture<'_> {
        Box::pin(async { self.0.recv().await.transpose() })
    }
}
#[derive(Default)]
struct Runner {
    calls: Mutex<Vec<Vec<String>>>,
    cause: Mutex<Option<Arc<OnceLock<IncompleteReason>>>>,
}
impl ProcessRunner for Runner {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, ProcessError>> + Send + 'a>,
    > {
        Box::pin(async move {
            if let Some(cause) = self.cause.lock().unwrap().as_ref() {
                assert_eq!(cause.get(), Some(&IncompleteReason::HostWindowOpened));
            }
            self.calls.lock().unwrap().push(
                std::iter::once(program.to_owned())
                    .chain(args.iter().map(|s| (*s).to_owned()))
                    .collect(),
            );
            Ok(String::new())
        })
    }
}
fn host(snapshots: Vec<Result<Vec<HostWindow>, ProcessError>>) -> Arc<Host> {
    Arc::new(Host {
        snapshots: Mutex::new(snapshots.into()),
        calls: Default::default(),
    })
}
fn stray() -> Result<Vec<HostWindow>, ProcessError> {
    Ok(vec![window("0xdef", 7, "aquamarine")])
}
fn spawn(host: Arc<Host>, runner: Arc<Runner>, events: Option<Box<dyn TakeoverEvents>>) -> Guard {
    Guard::spawn(
        host,
        runner,
        events,
        WindowPid(7),
        WindowAddress("0xabc".into()),
        99,
    )
}
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}
#[tokio::test(start_paused = true)]
async fn an_openwindow_event_with_a_stray_window_kills_the_sandbox_before_the_next_tick() {
    let host = host(vec![Ok(vec![]), stray()]);
    let runner = Arc::new(Runner::default());
    let (tx, rx) = mpsc::channel(4);
    let guard = spawn(host, runner.clone(), Some(Box::new(Events(rx))));
    settle().await;
    tx.send(Ok(HyprEvent::OpenWindow {
        address: "def".into(),
        workspace: "keek".into(),
        class: "ignored".into(),
    }))
    .await
    .unwrap();
    settle().await;
    assert_eq!(guard.cause(), Some(IncompleteReason::HostWindowOpened));
    tokio::time::advance(Duration::from_secs(5)).await;
    settle().await;
    assert_eq!(
        *runner.calls.lock().unwrap(),
        vec![vec!["kill", "-KILL", "99"]]
    );
}
#[tokio::test(start_paused = true)]
async fn the_cause_is_recorded_before_the_kill_runs() {
    let host = host(vec![stray()]);
    let runner = Arc::new(Runner::default());
    let guard = spawn(host, runner.clone(), None);
    *runner.cause.lock().unwrap() = Some(guard.cause.clone());
    settle().await;
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
}
#[tokio::test(start_paused = true)]
async fn a_class_change_without_an_event_is_caught_on_the_next_tick() {
    let host = host(vec![Ok(vec![]), Ok(vec![window("0xabc", 7, "foreign")])]);
    let runner = Arc::new(Runner::default());
    let guard = spawn(host, runner.clone(), None);
    settle().await;
    tokio::time::advance(Duration::from_millis(999)).await;
    settle().await;
    assert!(runner.calls.lock().unwrap().is_empty());
    tokio::time::advance(Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(guard.cause(), Some(IncompleteReason::HostWindowOpened));
}
#[tokio::test(start_paused = true)]
async fn a_failed_listing_does_not_kill_and_the_guard_keeps_watching() {
    let host = host(vec![
        Err(ProcessError::Failed {
            program: "fake".into(),
            message: "failed".into(),
        }),
        stray(),
    ]);
    let runner = Arc::new(Runner::default());
    let guard = spawn(host, runner.clone(), None);
    settle().await;
    assert!(runner.calls.lock().unwrap().is_empty());
    tokio::time::advance(CLASS_POLL).await;
    settle().await;
    assert_eq!(guard.cause(), Some(IncompleteReason::HostWindowOpened));
}
#[tokio::test(start_paused = true)]
async fn the_guard_keeps_polling_after_the_event_stream_ends() {
    let host = host(vec![Ok(vec![]), stray()]);
    let runner = Arc::new(Runner::default());
    let (tx, rx) = mpsc::channel(4);
    let guard = spawn(host, runner.clone(), Some(Box::new(Events(rx))));
    drop(tx);
    settle().await;
    assert!(runner.calls.lock().unwrap().is_empty());
    tokio::time::advance(CLASS_POLL).await;
    settle().await;
    assert_eq!(guard.cause(), Some(IncompleteReason::HostWindowOpened));
}
#[tokio::test(start_paused = true)]
async fn unrelated_events_do_not_trigger_a_listing() {
    let host = host(vec![Ok(vec![]), stray()]);
    let runner = Arc::new(Runner::default());
    let (tx, rx) = mpsc::channel(4);
    let guard = spawn(host.clone(), runner.clone(), Some(Box::new(Events(rx))));
    settle().await;
    tx.send(Ok(HyprEvent::Workspace("keek".into())))
        .await
        .unwrap();
    settle().await;
    assert_eq!(host.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(runner.calls.lock().unwrap().is_empty());
    drop(guard);
}
#[tokio::test(start_paused = true)]
async fn dropping_the_guard_stops_watching_and_never_kills() {
    let host = host(vec![Ok(vec![]), stray()]);
    let runner = Arc::new(Runner::default());
    let guard = spawn(host, runner.clone(), None);
    settle().await;
    drop(guard);
    tokio::time::advance(CLASS_POLL).await;
    settle().await;
    assert!(runner.calls.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_guard_keeps_polling_after_an_event_stream_error() {
    let host = host(vec![Ok(vec![]), stray()]);
    let runner = Arc::new(Runner::default());
    let (tx, rx) = mpsc::channel(4);
    let guard = spawn(host, runner.clone(), Some(Box::new(Events(rx))));
    tx.send(Err(ProcessError::Failed {
        program: "events".into(),
        message: "failed".into(),
    }))
    .await
    .unwrap();
    settle().await;
    assert!(runner.calls.lock().unwrap().is_empty());
    tokio::time::advance(CLASS_POLL).await;
    settle().await;
    assert_eq!(guard.cause(), Some(IncompleteReason::HostWindowOpened));
}
