use crate::checklist::IncompleteReason;
use crate::compositor::{
    Compositor, HostWindow, SESSION_WINDOW_CLASS, TakeoverEvents, WindowAddress, WindowPid,
};
use crate::hypr::HyprEvent;
use crate::process::ProcessRunner;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::task::JoinHandle;

const CLASS_POLL: Duration = Duration::from_secs(1);
fn stray_window<'a>(
    windows: &'a [HostWindow],
    pid: WindowPid,
    session: &WindowAddress,
) -> Option<&'a HostWindow> {
    windows.iter().find(|window| {
        window.pid == pid.0 && (window.address != *session || window.class != SESSION_WINDOW_CLASS)
    })
}
async fn watch(
    compositor: &dyn Compositor,
    mut events: Option<Box<dyn TakeoverEvents>>,
    pid: WindowPid,
    session: &WindowAddress,
) -> HostWindow {
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + CLASS_POLL, CLASS_POLL);
    let mut failure = false;
    loop {
        match compositor.windows().await {
            Ok(windows) => {
                failure = false;
                if let Some(window) = stray_window(&windows, pid, session) {
                    return window.clone();
                }
            }
            Err(error) => {
                if !failure {
                    tracing::warn!(%error, "Cannot list host windows; the host window guard will retry");
                }
                failure = true;
            }
        }
        loop {
            tokio::select! {
                _ = tick.tick() => break,
                event = async {
                    match events.as_mut() {
                        Some(events) => events.next().await,
                        None => std::future::pending().await,
                    }
                } => match event {
                    Ok(Some(HyprEvent::OpenWindow { .. })) => break,
                    Ok(Some(HyprEvent::Submap(_) | HyprEvent::Workspace(_) | HyprEvent::FocusedMonitor(_) | HyprEvent::MoveWorkspace { .. } | HyprEvent::CloseWindow(_) | HyprEvent::Custom(_))) => {},
                    Ok(None) => events = None,
                    Err(error) => {
                        tracing::warn!(%error, "Host window events stopped; the guard will keep polling");
                        events = None;
                    }
                }
            }
        }
    }
}
pub(crate) struct Guard {
    task: JoinHandle<()>,
    cause: Arc<OnceLock<IncompleteReason>>,
}
impl Guard {
    pub(crate) fn spawn(
        compositor: Arc<dyn Compositor>,
        runner: Arc<dyn ProcessRunner>,
        events: Option<Box<dyn TakeoverEvents>>,
        pid: WindowPid,
        session: WindowAddress,
        sandbox_pid: u32,
    ) -> Self {
        let cause = Arc::new(OnceLock::new());
        let recorded = cause.clone();
        let task = tokio::spawn(async move {
            let window = watch(compositor.as_ref(), events, pid, &session).await;
            tracing::warn!(class = %window.class, address = %window.address.0, "Session opened another host window; ending the session");
            let _ = recorded.set(IncompleteReason::HostWindowOpened);
            if let Err(error) = runner
                .output("kill", &["-KILL", &sandbox_pid.to_string()], None)
                .await
            {
                tracing::error!(%error, "Cannot kill the sandbox after it opened another host window");
            }
        });
        Self { task, cause }
    }
    pub(crate) fn cause(&self) -> Option<IncompleteReason> {
        self.cause.get().copied()
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[cfg(test)]
#[path = "window_guard_tests.rs"]
mod tests;
