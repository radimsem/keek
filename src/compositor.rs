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
    fn workspace(&self, places: usize) -> WorkspaceFuture<'_>;
    fn move_window(&self, _: WindowAddress, _: WindowSpot) -> CompositorFuture<'_> {
        Box::pin(async { Ok(()) })
    }
    fn window_events(&self) -> EventsFuture<'_>;
    fn install(&self) -> CompositorFuture<'_>;
    fn remove(&self) -> CompositorFuture<'_>;
    /// Moves a nested Hyprland the developer started, which the host rule caught, back to their active workspace.
    fn release_caught_windows(&self) -> CompositorFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSpot {
    Shown { x: i32, y: i32 },
    Hidden,
}
pub type WorkspaceFuture<'a> =
    Pin<Box<dyn Future<Output = Result<WorkspacePlacement, ProcessError>> + Send + 'a>>;
#[derive(Debug, Clone)]
pub struct WorkspacePlacement {
    pub layout: crate::slots::WorkspaceLayout,
    pub monitor: String,
    pub origin: (i32, i32),
}
#[derive(Debug, Clone)]
pub struct Placement {
    pub geometry: crate::slots::CardGeometry,
    pub monitor: String,
    pub origin: (i32, i32),
    pub initial: WindowSpot,
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
    async fn host_json(&self, request: &str) -> Result<serde_json::Value, ProcessError> {
        let reply = self
            .runner
            .output("hyprctl", &[request, "-j"], None)
            .await?;
        serde_json::from_str(&reply).map_err(|error| ProcessError::Failed {
            program: format!("hyprctl {request}"),
            message: format!("the reply is not JSON: {error}"),
        })
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
    fn workspace(&self, places: usize) -> WorkspaceFuture<'_> {
        Box::pin(crate::window_placement::workspace(places))
    }
    fn move_window(&self, window: WindowAddress, spot: WindowSpot) -> CompositorFuture<'_> {
        Box::pin(async move {
            let window = format!("address:{}", window.0);
            let lua = crate::window_placement::move_script(&window, spot);
            check_eval(self.runner.output("hyprctl", &["eval", &lua], None).await?)
        })
    }
    fn install(&self) -> CompositorFuture<'_> {
        Box::pin(async move {
            // Hyprland rules cannot match a pid, so this class rule also covers a nested Hyprland the developer starts; see docs/viewer.md.
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
    fn release_caught_windows(&self) -> CompositorFuture<'_> {
        Box::pin(async move {
            let clients = self.host_json("clients").await?;
            let mut caught = Vec::new();
            for client in clients.as_array().into_iter().flatten() {
                if client["class"] != SESSION_WINDOW_CLASS
                    || !matches!(
                        client["workspace"]["name"].as_str(),
                        Some("keek" | "keek-rail")
                    )
                {
                    continue;
                }
                let (Some(address), Some(pid)) =
                    (client["address"].as_str(), client["pid"].as_u64())
                else {
                    continue;
                };
                // A Session Window belongs to a keek process, so this leaves another daemon's alone.
                let owner = self
                    .runner
                    .output("ps", &["-o", "comm=", "-p", &pid.to_string()], None)
                    .await;
                if owner.is_ok_and(|name| name.trim() == "Hyprland") {
                    caught.push(address);
                }
            }
            if caught.is_empty() {
                return Ok(());
            }
            let active = self.host_json("activeworkspace").await?;
            let name = active["name"].as_str().unwrap_or_default();
            // A window tiled on the Agents Workspace would cover its Session Cards.
            if name == "keek" {
                return Ok(());
            }
            // A named workspace has a negative id and is selected as name:<name>.
            let workspace = match active["id"].as_i64() {
                Some(id) if id > 0 => id.to_string(),
                Some(_) | None => {
                    format!("name:{}", name.replace('\\', "\\\\").replace('\'', "\\'"))
                }
            };
            // Disabling the rule undoes no_focus but not float, size or workspace, which Hyprland applies once at open.
            let script = caught
                .iter()
                .map(|address| {
                    format!(
                        "hl.dispatch(hl.dsp.window.move({{ window = 'address:{address}', workspace = '{workspace}', follow = false }})); hl.dispatch(hl.dsp.window.float({{ window = 'address:{address}', action = 'disable' }}))"
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            let output = self
                .runner
                .output("hyprctl", &["eval", &script], None)
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
    struct Host {
        hyprctl: Mutex<Vec<Vec<String>>>,
        clients: &'static str,
        active: &'static str,
    }
    impl ProcessRunner for Host {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            _: Option<&'a std::path::Path>,
        ) -> Pin<Box<dyn Future<Output = Result<String, ProcessError>> + Send + 'a>> {
            Box::pin(async move {
                if program == "ps" {
                    // Pid 7 is a Hyprland the developer started, pid 8 a keek process, and any other pid has exited.
                    return match args[3] {
                        "7" => Ok("Hyprland\n".into()),
                        "8" => Ok("keek\n".into()),
                        _ => Err(ProcessError::Failed {
                            program: "ps".into(),
                            message: "exit status: 1".into(),
                        }),
                    };
                }
                self.hyprctl
                    .lock()
                    .unwrap()
                    .push(args.iter().map(|arg| (*arg).to_owned()).collect());
                Ok(match args[0] {
                    "clients" => self.clients,
                    "activeworkspace" => self.active,
                    _ => "ok",
                }
                .into())
            })
        }
    }
    const CAUGHT: &str =
        r#"[{"address":"0xaaa","pid":7,"class":"aquamarine","workspace":{"name":"keek"}}]"#;
    async fn release(
        clients: &'static str,
        active: &'static str,
    ) -> (Result<(), ProcessError>, Vec<Vec<String>>) {
        let host = Arc::new(Host {
            hyprctl: Mutex::new(Vec::new()),
            clients,
            active,
        });
        let result = HostCompositor::new(host.clone())
            .release_caught_windows()
            .await;
        let calls = host.hyprctl.lock().unwrap().clone();
        (result, calls)
    }
    #[tokio::test]
    async fn releasing_tiles_only_caught_windows_on_the_active_workspace() {
        let (result, calls) = release(
            r#"[{"address":"0xaaa","pid":7,"class":"aquamarine","workspace":{"name":"keek"}},
                {"address":"0xbbb","pid":7,"class":"aquamarine","workspace":{"name":"2"}},
                {"address":"0xccc","pid":7,"class":"kitty","workspace":{"name":"keek"}},
                {"address":"0xddd","pid":7,"class":"aquamarine","workspace":{"name":"keek-rail"}}]"#,
            r#"{"id":3,"name":"3"}"#,
        )
        .await;
        result.unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls[2],
            [
                "eval",
                "hl.dispatch(hl.dsp.window.move({ window = 'address:0xaaa', workspace = '3', follow = false })); hl.dispatch(hl.dsp.window.float({ window = 'address:0xaaa', action = 'disable' })); hl.dispatch(hl.dsp.window.move({ window = 'address:0xddd', workspace = '3', follow = false })); hl.dispatch(hl.dsp.window.float({ window = 'address:0xddd', action = 'disable' }))"
            ]
        );
    }
    #[tokio::test]
    async fn only_a_window_owned_by_a_hyprland_process_is_released() {
        let (result, calls) = release(
            r#"[{"address":"0xaaa","pid":8,"class":"aquamarine","workspace":{"name":"keek"}},
                {"address":"0xbbb","pid":9,"class":"aquamarine","workspace":{"name":"keek"}}]"#,
            r#"{"id":3,"name":"3"}"#,
        )
        .await;
        result.unwrap();
        assert_eq!(calls, [["clients", "-j"]]);
    }
    #[tokio::test]
    async fn releasing_with_nothing_caught_dispatches_nothing() {
        let (result, calls) = release(
            r#"[{"address":"0xccc","pid":7,"class":"kitty","workspace":{"name":"keek"}}]"#,
            r#"{"id":3,"name":"3"}"#,
        )
        .await;
        result.unwrap();
        assert_eq!(calls, [["clients", "-j"]]);
    }
    #[tokio::test]
    async fn a_caught_window_stays_put_while_the_agents_workspace_is_active() {
        let (result, calls) = release(CAUGHT, r#"{"id":-98,"name":"keek"}"#).await;
        result.unwrap();
        assert_eq!(calls, [["clients", "-j"], ["activeworkspace", "-j"]]);
    }
    #[tokio::test]
    async fn a_named_active_workspace_is_selected_by_its_quoted_name() {
        let (result, calls) = release(CAUGHT, r#"{"id":-97,"name":"d\\e's"}"#).await;
        result.unwrap();
        assert!(
            calls[2][1].contains(r"workspace = 'name:d\\e\'s', follow = false"),
            "{}",
            calls[2][1]
        );
    }
    #[tokio::test]
    async fn an_unreadable_window_list_is_reported() {
        let (result, calls) = release("no socket", "{}").await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("hyprctl clients"), "{error}");
        assert_eq!(calls.len(), 1);
    }
    #[tokio::test]
    async fn a_shown_window_moves_to_the_agents_workspace_and_a_hidden_one_out_of_sight() {
        let runner = Arc::new(Runner(Mutex::new(Vec::new()), "ok"));
        let compositor = HostCompositor::new(runner.clone());
        compositor
            .move_window(
                WindowAddress("0xabc".into()),
                WindowSpot::Shown { x: 24, y: 50 },
            )
            .await
            .unwrap();
        compositor
            .move_window(WindowAddress("0xabc".into()), WindowSpot::Hidden)
            .await
            .unwrap();
        let calls = runner.0.lock().unwrap();
        assert_eq!(
            calls[0],
            [
                "hyprctl",
                "eval",
                "hl.dispatch(hl.dsp.window.move({ window = 'address:0xabc', workspace = 'name:keek', follow = false })); hl.dispatch(hl.dsp.window.move({ window = 'address:0xabc', x = 24, y = 50, relative = false }))"
            ]
        );
        assert_eq!(
            calls[1],
            [
                "hyprctl",
                "eval",
                "hl.dispatch(hl.dsp.window.move({ window = 'address:0xabc', workspace = 'name:keek-rail', follow = false }))"
            ]
        );
    }
    #[tokio::test]
    async fn a_refused_window_move_is_reported() {
        let runner = Arc::new(Runner(Mutex::new(Vec::new()), "no such window"));
        let error = HostCompositor::new(runner)
            .move_window(WindowAddress("0xabc".into()), WindowSpot::Hidden)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("no such window"), "{error}");
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
