use super::*;
use crate::paths::Paths;
use std::fs::{DirBuilder, File, TryLockError};
use std::os::unix::fs::DirBuilderExt;

pub async fn run(
    paths: &Paths,
    idle_limit: Duration,
    clock: impl Clock + Send + Sync + 'static,
) -> Result<Exit, DaemonError> {
    run_with_viewer(
        paths,
        idle_limit,
        clock,
        std::sync::Arc::new(crate::viewer::NoopViewer),
    )
    .await
}
pub async fn run_with_viewer(
    paths: &Paths,
    idle_limit: Duration,
    clock: impl Clock + Send + Sync + 'static,
    viewer: std::sync::Arc<dyn crate::viewer::Viewer>,
) -> Result<Exit, DaemonError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(paths.runtime_dir())
        .map_err(DaemonError::RuntimeDir)?;

    // The kernel releases this lock when the process ends, however it ends, so a
    // socket left by a crash is safe to replace once the lock is held.
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.lock())
        .map_err(DaemonError::Lock)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(Exit::AlreadyRunning),
        Err(TryLockError::Error(error)) => return Err(DaemonError::Lock(error)),
    }

    let socket = paths.socket();
    remove_socket(&socket)?;
    let listener = UnixListener::bind(&socket).map_err(DaemonError::Bind)?;

    let config = crate::config::Config::read(paths.config_file())
        .await
        .map_err(DaemonError::Config)?;
    let runner: std::sync::Arc<dyn crate::process::ProcessRunner> =
        std::sync::Arc::new(crate::process::HostRunner);
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let compositor: std::sync::Arc<dyn crate::compositor::Compositor> =
        std::sync::Arc::new(crate::compositor::HostCompositor::new(runner.clone()));
    let dependencies = crate::registry::Dependencies {
        viewer,
        sessions_dir: paths.sessions_dir().to_owned(),
        projects_config: paths.projects_config().to_owned(),
        projects_cache: paths.projects_cache().to_owned(),
        trees: paths.trees().to_owned(),
        cap: config.session_cap,
        host: crate::session::HostEnvironment::capture(),
        clock: std::sync::Arc::new(clock),
        runner: runner.clone(),
        compositor: compositor.clone(),
        factory: std::sync::Arc::new(crate::session_factory::SandboxFactory {
            driver: crate::driver::driver_path(&home),
            runner,
            compositor,
            projects_cache: paths.projects_cache().to_owned(),
            trees: paths.trees().to_owned(),
        }),
    };
    let (catalog_sender, catalog_receiver) = tokio::sync::watch::channel(None);
    let mut registry = crate::registry::Registry::new(dependencies, catalog_receiver);
    let runtime = paths.runtime_dir().to_owned();
    registry.background.spawn(async move {
        let result = crate::catalog::load(&runtime, &home)
            .await
            .map(std::sync::Arc::new)
            .map_err(|error| error.to_string());
        catalog_sender.send_replace(Some(result));
    });
    let exit = serve(
        listener,
        IdleWatch::new(crate::clock::MonotonicClock, idle_limit),
        registry,
    )
    .await;
    let removed = remove_socket(&socket);
    let exit = exit?;
    removed?;
    Ok(exit)
}
