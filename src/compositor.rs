use crate::process::{ProcessError, ProcessRunner};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type CompositorFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ProcessError>> + Send + 'a>>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPid(pub u32);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowAddress(pub String);
pub type WindowFuture<'a> =
    Pin<Box<dyn Future<Output = Result<WindowAddress, ProcessError>> + Send + 'a>>;
pub const SESSION_WINDOW_CLASS: &str = "aquamarine";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWindow {
    pub address: WindowAddress,
    pub pid: u32,
    pub class: String,
}
pub type WindowsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<HostWindow>, ProcessError>> + Send + 'a>>;
pub trait Compositor: Send + Sync {
    fn windows(&self) -> WindowsFuture<'_> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn takeover_events(&self) -> TakeoverEventsFuture<'_> {
        Box::pin(async {
            Ok(self
                .window_events()
                .await?
                .map(|events| Box::new(events) as Box<dyn TakeoverEvents>))
        })
    }
    fn place_window(
        &self,
        pid: Option<WindowPid>,
        slot: Placement,
        events: Option<WindowEvents>,
    ) -> WindowFuture<'_>;
    fn slot(
        &self,
        index: crate::viewer::SlotId,
        count: usize,
        size: Option<crate::project::WindowSize>,
    ) -> SlotFuture<'_>;
    fn window_events(&self) -> EventsFuture<'_>;
    fn install(&self) -> CompositorFuture<'_>;
    fn remove(&self) -> CompositorFuture<'_>;
}
pub type SlotFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Placement, ProcessError>> + Send + 'a>>;
#[derive(Debug, Clone)]
pub struct Placement {
    pub slot: crate::slots::Slot,
    pub monitor: String,
    pub origin: (i32, i32),
}
pub type WindowEvents = tokio::io::Lines<tokio::io::BufReader<tokio::net::UnixStream>>;
pub type EventsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<WindowEvents>, ProcessError>> + Send + 'a>>;
pub struct HostCompositor {
    runner: Arc<dyn ProcessRunner>,
}
impl HostCompositor {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
    }
}
impl Compositor for HostCompositor {
    fn windows(&self) -> WindowsFuture<'_> {
        Box::pin(crate::window_placement::windows())
    }
    fn window_events(&self) -> EventsFuture<'_> {
        Box::pin(async { crate::hypr::subscribe().await.map(Some) })
    }
    fn place_window(
        &self,
        pid: Option<WindowPid>,
        slot: Placement,
        events: Option<WindowEvents>,
    ) -> WindowFuture<'_> {
        Box::pin(crate::window_placement::place(pid, slot, events))
    }
    fn slot(
        &self,
        index: crate::viewer::SlotId,
        count: usize,
        size: Option<crate::project::WindowSize>,
    ) -> SlotFuture<'_> {
        Box::pin(crate::window_placement::slot(index, count, size))
    }
    fn install(&self) -> CompositorFuture<'_> {
        Box::pin(async move {
            let output = self.runner.output("hyprctl", &["eval", "if keek_session_rule then keek_session_rule:set_enabled(false) end; keek_session_rule = hl.window_rule({ name = 'keek-session-window', match = { class = '^aquamarine$' }, float = true, size = { 1280, 800 }, workspace = 'name:keek silent', no_initial_focus = true, no_focus = true, render_unfocused = true }); keek_session_rule:set_enabled(true)"], None).await?;
            check_eval(output)
        })
    }
    fn remove(&self) -> CompositorFuture<'_> {
        Box::pin(async move {
            let output = self
                .runner
                .output(
                    "hyprctl",
                    &[
                        "eval",
                        "if keek_session_rule then keek_session_rule:set_enabled(false) end",
                    ],
                    None,
                )
                .await?;
            check_eval(output)
        })
    }
}

fn check_eval(output: String) -> Result<(), ProcessError> {
    if output.trim() == "ok" {
        Ok(())
    } else {
        Err(ProcessError::Failed {
            program: "hyprctl eval".into(),
            message: output,
        })
    }
}

pub type EventFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<crate::hypr::HyprEvent>, ProcessError>> + Send + 'a>>;
pub trait TakeoverEvents: Send {
    fn next(&mut self) -> EventFuture<'_>;
}
pub type TakeoverEventsFuture<'a> = Pin<
    Box<dyn Future<Output = Result<Option<Box<dyn TakeoverEvents>>, ProcessError>> + Send + 'a>,
>;
impl TakeoverEvents for WindowEvents {
    fn next(&mut self) -> EventFuture<'_> {
        Box::pin(async move {
            loop {
                let line = self.next_line().await.map_err(|source| ProcessError::Io {
                    program: "Hyprland events".into(),
                    source,
                })?;
                let Some(line) = line else {
                    return Ok(None);
                };
                if let Some(event) = crate::hypr::parse(&line) {
                    return Ok(Some(event));
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Runner(Mutex<Vec<Vec<String>>>, &'static str);
    impl ProcessRunner for Runner {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            _: Option<&'a std::path::Path>,
        ) -> Pin<Box<dyn Future<Output = Result<String, ProcessError>> + Send + 'a>> {
            Box::pin(async move {
                self.0.lock().unwrap().push(
                    std::iter::once(program.to_owned())
                        .chain(args.iter().map(|arg| (*arg).to_owned()))
                        .collect(),
                );
                Ok(self.1.into())
            })
        }
    }
    #[tokio::test]
    async fn the_host_rule_keeps_session_windows_unfocused_without_changing_config() {
        let runner = Arc::new(Runner(Mutex::new(Vec::new()), "ok"));
        let compositor = HostCompositor::new(runner.clone());
        compositor.install().await.unwrap();
        compositor.remove().await.unwrap();
        let calls = runner.0.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(&calls[0][..2], ["hyprctl", "eval"]);
        let install = &calls[0][2];
        for property in [
            "class = '^aquamarine$'",
            "float = true",
            "size = { 1280, 800 }",
            "workspace = 'name:keek silent'",
            "no_initial_focus = true",
            "no_focus = true",
            "render_unfocused = true",
        ] {
            assert!(install.contains(property), "missing {property}: {install}");
        }
        assert!(!install.contains("submap"));
        assert!(calls[1][2].contains(":set_enabled(false)"));
    }
    #[tokio::test]
    async fn a_rejected_rule_is_reported() {
        let runner = Arc::new(Runner(Mutex::new(Vec::new()), "rule rejected"));
        let error = HostCompositor::new(runner)
            .install()
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("rule rejected"), "{error}");
    }
}
