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

    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(DaemonError::Signal)?;
    let terminated = async move {
        if signal.recv().await.is_none() {
            std::future::pending::<()>().await;
        }
    };

    crate::session_frame::clear(paths.runtime_dir()).await;
    remove_bar_state(&paths.bar_state());
    install_icons(paths.icon_theme()).await;
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
    let bar = crate::viewer::bar_viewer::BarStateViewer::new(viewer, paths.bar_state());
    let viewer: std::sync::Arc<dyn crate::viewer::Viewer> = bar.clone();
    let dependencies = crate::registry::Dependencies {
        frames_dir: paths.runtime_dir().join("frames"),
        journals_dir: paths.state_dir().join("sessions"),
        session_idle_limit: session_idle_limit(
            std::env::var(SESSION_IDLE_LIMIT_ENV).ok().as_deref(),
        )?,
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
            toolchains: paths.toolchains().to_owned(),
            mise: std::sync::Arc::new(crate::toolchain::HostMise),
            prepare: paths.prepare().to_owned(),
        }),
        delegate: decision_model(&config),
    };
    let (catalog_sender, catalog_receiver) = tokio::sync::watch::channel(None);
    let mut registry = crate::registry::Registry::new(dependencies, catalog_receiver);
    registry.recover().await;
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
        terminated,
    )
    .await;
    // The caller shuts down the given viewer, so only the bar writer stops here.
    bar.stop_writer().await;
    remove_bar_state(&paths.bar_state());
    let removed = remove_socket(&socket);
    let exit = exit?;
    removed?;
    Ok(exit)
}

/// A client that cannot be built leaves `delegate_step` unlisted and the daemon running.
fn decision_model(config: &crate::config::Config) -> Option<crate::session::delegate::Delegate> {
    let model = config.decision_model.as_ref()?;
    match crate::decision_model::JevClient::new(model) {
        Ok(client) => Some(crate::session::delegate::Delegate {
            model: std::sync::Arc::new(client),
            config: model.clone(),
        }),
        Err(error) => {
            tracing::error!(%error, "delegate_step stays off");
            None
        }
    }
}

async fn install_icons(theme: std::path::PathBuf) {
    match tokio::task::spawn_blocking(move || crate::icons::install(&theme)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "keek icon was not installed"),
        Err(error) => tracing::warn!(%error, "keek icon install stopped unexpectedly"),
    }
}

fn remove_bar_state(path: &std::path::Path) {
    if let Err(error) = crate::bar_state::remove(path) {
        tracing::warn!(%error, "bar state could not be removed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DecisionModelConfig};

    fn config(api_key_env: Option<&str>) -> Config {
        Config {
            decision_model: Some(DecisionModelConfig {
                endpoint: "http://127.0.0.1:9".into(),
                model: "jev-test".into(),
                api_key_env: api_key_env.map(str::to_owned),
                threshold: 0.9,
                timeout_ms: 1500,
            }),
            ..Config::default()
        }
    }

    #[test]
    fn a_config_without_a_decision_model_builds_no_client() {
        assert!(decision_model(&Config::default()).is_none());
    }

    #[test]
    fn a_local_decision_model_needs_no_key() {
        assert!(decision_model(&config(None)).is_some());
        assert!(decision_model(&config(Some(""))).is_some());
    }

    #[test]
    fn a_decision_model_whose_key_variable_is_unset_stays_off() {
        assert!(decision_model(&config(Some("KEEK_TEST_UNSET_KEY_VARIABLE"))).is_none());
    }

    #[tokio::test]
    async fn startup_clears_old_frames_after_acquiring_the_lock() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::under_root(root.path());
        let frames = paths.runtime_dir().join("frames");
        std::fs::create_dir_all(&frames).unwrap();
        std::fs::write(frames.join("old.png"), b"old").unwrap();
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(paths.lock())
            .unwrap();
        lock.try_lock().unwrap();
        assert!(matches!(
            run(&paths, Duration::from_secs(1), crate::clock::MonotonicClock).await,
            Ok(Exit::AlreadyRunning)
        ));
        assert!(frames.join("old.png").exists());
    }

    #[tokio::test]
    async fn startup_removes_a_stale_bar_file_after_acquiring_the_lock() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::under_root(root.path());
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();
        std::fs::write(paths.bar_state(), b"stale").unwrap();
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(paths.lock())
            .unwrap();
        lock.try_lock().unwrap();
        assert!(matches!(
            run(&paths, Duration::from_secs(1), crate::clock::MonotonicClock).await,
            Ok(Exit::AlreadyRunning)
        ));
        assert!(paths.bar_state().exists());

        // A child another test forks meanwhile shares this lock until it execs, so closing alone keeps it.
        lock.unlock().unwrap();
        std::fs::create_dir_all(paths.config_file().parent().unwrap()).unwrap();
        std::fs::write(paths.config_file(), "session_cap = 0").unwrap();
        assert!(matches!(
            run(&paths, Duration::from_secs(1), crate::clock::MonotonicClock).await,
            Err(DaemonError::Config(_))
        ));
        assert!(!paths.bar_state().exists());
    }

    #[tokio::test]
    async fn the_bar_file_is_removed_when_the_daemon_stops() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::under_root(root.path());
        let socket = paths.socket();
        let bar = paths.bar_state();
        let stopped = run(&paths, Duration::from_secs(1), crate::clock::MonotonicClock);
        let publish_while_running = async {
            while !socket.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            std::fs::write(&bar, b"{}").unwrap();
            assert!(bar.exists());
        };

        let (exit, ()) = tokio::join!(stopped, publish_while_running);

        assert!(matches!(exit, Ok(Exit::Idle)));
        assert!(!paths.bar_state().exists());
    }
}
