use crate::driver::{CuaDriver, Driver, DriverError, DriverFuture};
use crate::process::ProcessRunner;
use crate::session::{DriverFactory, SessionError};
use crate::session_folder::SessionFolder;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub struct SandboxFactory {
    pub driver: PathBuf,
    pub runner: Arc<dyn ProcessRunner>,
    pub start_command: Option<String>,
}
impl DriverFactory for SandboxFactory {
    fn open<'a>(
        &'a self,
        folder: &'a SessionFolder,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Box<dyn Driver>, SessionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let folder_copy = folder.clone();
            let driver = self.driver.clone();
            let start = self.start_command.clone();
            let mut command = tokio::task::spawn_blocking(move || {
                let spec = crate::sandbox::SandboxSpec::from_env(folder_copy.root(), &driver)?;
                spec.prepare(start.as_deref())?;
                spec.command()
            })
            .await
            .map_err(|e| SessionError::Refused(e.to_string()))??;
            // PDEATHSIG follows the spawning thread, so spawn on the runtime, never in spawn_blocking.
            let mut child = command.spawn().map_err(SessionError::SpawnPasta)?;
            let reader = child.stdout.take().ok_or_else(|| {
                SessionError::Refused("Cua Driver has no stdout. Check the sandbox command.".into())
            })?;
            let writer = child.stdin.take().ok_or_else(|| {
                SessionError::Refused("Cua Driver has no stdin. Check the sandbox command.".into())
            })?;
            let connected =
                tokio::time::timeout(Duration::from_secs(30), CuaDriver::connect(reader, writer))
                    .await;
            let driver = match connected {
                Ok(Ok(driver)) => driver,
                other => {
                    let _ = child.kill().await;
                    return Err(SessionError::Refused(format!(
                        "Agent Session desktop did not become ready: {}. Read {}, {}, {} and {} before retrying.",
                        match other {
                            Ok(Err(e)) => e.to_string(),
                            Err(e) => e.to_string(),
                            Ok(Ok(_)) => String::new(),
                        },
                        folder.root().join("session.err").display(),
                        folder.root().join("hyprland.log").display(),
                        folder.root().join("sandbox.err").display(),
                        folder.root().join("driver.log").display()
                    )));
                }
            };
            let mut sandbox = SandboxDriver { driver, child };
            if let Err(error) = check_plugin(folder, self.runner.as_ref()).await {
                let _ = sandbox.close().await;
                return Err(error);
            }
            Ok(Box::new(sandbox) as Box<dyn Driver>)
        })
    }
}
async fn check_plugin(
    folder: &SessionFolder,
    runner: &dyn ProcessRunner,
) -> Result<(), SessionError> {
    let folder = folder.clone();
    let (plugins, version) = tokio::task::spawn_blocking(move || {
        let read = |name: &str| {
            crate::session_folder::read_sandbox_file(&folder.root().join(name)).map_err(|error| {
                SessionError::Refused(format!(
                    "Cannot read {name}: {error}. Check the Agent Session folder and retry."
                ))
            })
        };
        Ok::<_, SessionError>((read("plugins.json")?, read("hyprland-version.json")?))
    })
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))??;
    let plugins = serde_json::from_slice::<Value>(&plugins).ok();
    let version = serde_json::from_slice::<Value>(&version)
        .ok()
        .and_then(|value| {
            value
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".into());
    if plugins
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry.get("name").and_then(Value::as_str) == Some("cua-hyprland-plugin")
            })
        })
    {
        return Ok(());
    }
    let package = runner
        .output("pacman", &["-Q", "cua-hyprland-plugin"], None)
        .await
        .unwrap_or_else(|e| e.to_string());
    Err(SessionError::Refused(format!(
        "Agent Session refused because cua-hyprland-plugin did not load. Hyprland: {version}; plugin package: {package}. Install matching Hyprland and plugin packages, then retry."
    )))
}
struct SandboxDriver {
    driver: CuaDriver,
    child: tokio::process::Child,
}
impl Driver for SandboxDriver {
    fn list_tools(&mut self) -> DriverFuture<'_, Vec<rmcp::model::Tool>> {
        self.driver.list_tools()
    }
    fn call_tool<'a>(&'a mut self, name: &'a str, args: Value) -> DriverFuture<'a, Value> {
        self.driver.call_tool(name, args)
    }
    fn close(&mut self) -> DriverFuture<'_, ()> {
        Box::pin(async move {
            match tokio::time::timeout(Duration::from_secs(30), async {
                let close = self.driver.close().await;
                self.child
                    .wait()
                    .await
                    .map_err(|error| DriverError::Close(error.to_string()))?;
                close
            })
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    self.child
                        .kill()
                        .await
                        .map_err(|error| DriverError::Close(error.to_string()))?;
                    Err(DriverError::Close("timed out after 30 s".into()))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ProcessError;
    use std::path::Path;

    struct FakeRunner;
    impl ProcessRunner for FakeRunner {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            _: Option<&'a Path>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, ProcessError>> + Send + 'a>,
        > {
            Box::pin(async move {
                assert_eq!(program, "pacman");
                assert_eq!(args, ["-Q", "cua-hyprland-plugin"]);
                Ok("cua-hyprland-plugin 0.38.0-7".into())
            })
        }
    }
    fn folder(root: &Path, plugins: &str) -> SessionFolder {
        let name = "app_12-00-00".parse().unwrap();
        let folder = SessionFolder::new(root, &name);
        std::fs::create_dir_all(folder.root()).unwrap();
        std::fs::write(folder.root().join("plugins.json"), plugins).unwrap();
        std::fs::write(
            folder.root().join("hyprland-version.json"),
            r#"{"version":"0.56.2"}"#,
        )
        .unwrap();
        folder
    }
    #[tokio::test]
    async fn a_plugin_that_did_not_load_names_both_versions() {
        let root = tempfile::tempdir().unwrap();
        let folder = folder(root.path(), "[]");
        let error = check_plugin(&folder, &FakeRunner)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("0.56.2"), "{error}");
        assert!(error.contains("0.38.0-7"), "{error}");
    }
    #[tokio::test]
    async fn an_empty_plugin_probe_still_names_both_versions() {
        let root = tempfile::tempdir().unwrap();
        let folder = folder(root.path(), "");
        let error = check_plugin(&folder, &FakeRunner)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("0.56.2"), "{error}");
        assert!(error.contains("0.38.0-7"), "{error}");
    }
    #[tokio::test]
    async fn a_loaded_plugin_allows_the_session() {
        let root = tempfile::tempdir().unwrap();
        let folder = folder(root.path(), r#"[{"name":"cua-hyprland-plugin"}]"#);
        check_plugin(&folder, &FakeRunner).await.unwrap();
    }
    #[tokio::test]
    async fn probe_refuses_symlinks_and_names_the_file() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let folder = folder(root.path(), "[]");
        let path = folder.root().join("plugins.json");
        std::fs::remove_file(&path).unwrap();
        symlink(root.path().join("target"), &path).unwrap();
        std::fs::write(root.path().join("target"), "[]").unwrap();
        let error = check_plugin(&folder, &FakeRunner)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("plugins.json"), "{error}");
    }
    #[tokio::test]
    async fn probe_refuses_fifos_and_names_the_file() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let folder = folder(root.path(), "[]");
        let path = folder.root().join("hyprland-version.json");
        std::fs::remove_file(&path).unwrap();
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let error = check_plugin(&folder, &FakeRunner)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("hyprland-version.json"), "{error}");
    }
    #[tokio::test]
    async fn plugin_refusal_only_includes_the_version_field() {
        let root = tempfile::tempdir().unwrap();
        let folder = folder(root.path(), "[]");
        std::fs::write(
            folder.root().join("hyprland-version.json"),
            r#"{"version":"0.56.2","private":"must-not-appear"}"#,
        )
        .unwrap();
        let error = check_plugin(&folder, &FakeRunner)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("0.56.2") && error.contains("0.38.0-7"),
            "{error}"
        );
        assert!(!error.contains("must-not-appear"), "{error}");
    }
    #[tokio::test]
    async fn oversized_probe_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        let folder = folder(root.path(), "[]");
        std::fs::write(
            folder.root().join("hyprland-version.json"),
            format!(
                "{{\"version\":\"0.56.2\",\"padding\":\"{}\"}}",
                "x".repeat(70_000)
            ),
        )
        .unwrap();
        let error = check_plugin(&folder, &FakeRunner)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.len() < 2000, "refusal exposed oversized content");
    }
}
