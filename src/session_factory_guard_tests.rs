use super::*;
use crate::checklist::IncompleteReason;
use crate::compositor::*;
use rmcp::ServiceExt;
use std::sync::Mutex;
struct Server;
impl rmcp::ServerHandler for Server {}
struct Host(Arc<Mutex<Vec<HostWindow>>>);
impl Compositor for Host {
    fn windows(&self) -> WindowsFuture<'_> {
        let windows = self.0.lock().unwrap().clone();
        Box::pin(async { Ok(windows) })
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
struct Runner(Arc<Mutex<usize>>);
impl ProcessRunner for Runner {
    fn output<'a>(
        &'a self,
        _: &'a str,
        _: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<String, crate::process::ProcessError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            *self.0.lock().unwrap() += 1;
            Ok(String::new())
        })
    }
}
async fn fixture(
    windows: Arc<Mutex<Vec<HostWindow>>>,
    kills: Arc<Mutex<usize>>,
) -> (SandboxDriver, tokio::task::JoinHandle<()>) {
    let (near, far) = tokio::io::duplex(8192);
    let server = tokio::spawn(async {
        Server.serve(far).await.unwrap().waiting().await.unwrap();
    });
    let (read, write) = tokio::io::split(near);
    let driver = CuaDriver::connect(read, write).await.unwrap();
    let child = tokio::process::Command::new("true").spawn().unwrap();
    let guard = crate::window_guard::Guard::spawn(
        Arc::new(Host(windows)),
        Arc::new(Runner(kills)),
        None,
        WindowPid(7),
        WindowAddress("0xabc".into()),
        99,
    );
    (
        SandboxDriver {
            driver,
            child,
            guard: Some(guard),
            window: None,
        },
        server,
    )
}
fn stray() -> HostWindow {
    HostWindow {
        address: WindowAddress("0xdef".into()),
        pid: 7,
        class: "aquamarine".into(),
    }
}
#[tokio::test]
async fn a_sandbox_driver_reports_the_fired_guards_cause() {
    let windows = Arc::new(Mutex::new(vec![stray()]));
    let kills = Arc::new(Mutex::new(0));
    let (mut driver, server) = fixture(windows, kills.clone()).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(driver.end_cause(), Some(IncompleteReason::HostWindowOpened));
    assert_eq!(*kills.lock().unwrap(), 1);
    driver.close().await.unwrap();
    assert_eq!(driver.end_cause(), None);
    server.await.unwrap();
}
#[tokio::test]
async fn closing_the_driver_stops_the_guard_and_never_kills() {
    let windows = Arc::new(Mutex::new(vec![]));
    let kills = Arc::new(Mutex::new(0));
    let (mut driver, server) = fixture(windows.clone(), kills.clone()).await;
    driver.child.wait().await.unwrap();
    tokio::time::pause();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    driver.close().await.unwrap();
    *windows.lock().unwrap() = vec![stray()];
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(*kills.lock().unwrap(), 0);
    server.await.unwrap();
}
