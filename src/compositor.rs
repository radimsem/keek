use crate::process::{ProcessError, ProcessRunner};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type CompositorFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ProcessError>> + Send + 'a>>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPid(pub u32);
pub trait Compositor: Send + Sync {
    fn resize_window(
        &self,
        pid: WindowPid,
        size: crate::project::WindowSize,
    ) -> CompositorFuture<'_>;
    fn install(&self) -> CompositorFuture<'_>;
    fn remove(&self) -> CompositorFuture<'_>;
}
pub struct HostCompositor {
    runner: Arc<dyn ProcessRunner>,
}
impl HostCompositor {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
    }
}
impl Compositor for HostCompositor {
    fn resize_window(
        &self,
        pid: WindowPid,
        size: crate::project::WindowSize,
    ) -> CompositorFuture<'_> {
        Box::pin(async move {
            let pid = pid.0;
            let width = size.width.get();
            let height = size.height.get();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let output = self
                    .runner
                    .output("hyprctl", &["clients", "-j"], None)
                    .await?;
                let clients = serde_json::from_str::<serde_json::Value>(&output).map_err(|e| {
                    ProcessError::Failed {
                        program: "hyprctl clients".into(),
                        message: format!("Invalid client list: {e}"),
                    }
                })?;
                if clients.as_array().is_some_and(|clients| {
                    clients.iter().any(|c| {
                        c.get("pid").and_then(serde_json::Value::as_u64) == Some(u64::from(pid))
                    })
                }) {
                    let eval = format!(
                        "hl.dispatch(hl.dsp.window.resize({{ window = 'pid:{pid}', x = {width}, y = {height}, relative = false }}))"
                    );
                    return check_eval(
                        self.runner
                            .output("hyprctl", &["eval", &eval], None)
                            .await?,
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            }).await.map_err(|_| ProcessError::Failed {
                program: "hyprctl resize".into(),
                message: "The Session Window did not appear within 5 s (see sandbox.err)".into(),
            })?
        })
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
    async fn project_window_resize_names_the_pid_and_absolute_size() {
        struct ResizeRunner(Mutex<Vec<String>>);
        impl ProcessRunner for ResizeRunner {
            fn output<'a>(
                &'a self,
                _: &'a str,
                args: &'a [&'a str],
                _: Option<&'a std::path::Path>,
            ) -> crate::process::ProcessFuture<'a> {
                Box::pin(async move {
                    self.0.lock().unwrap().push(args.join(" "));
                    Ok(if args[0] == "clients" {
                        "[{\"pid\":123}]".into()
                    } else {
                        "ok".into()
                    })
                })
            }
        }
        let runner = Arc::new(ResizeRunner(Mutex::new(vec![])));
        HostCompositor::new(runner.clone())
            .resize_window(
                WindowPid(123),
                crate::project::WindowSize {
                    width: 390.try_into().unwrap(),
                    height: 844.try_into().unwrap(),
                },
            )
            .await
            .unwrap();
        assert!(
            runner
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.contains("window = 'pid:123', x = 390, y = 844, relative = false"))
        );
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
