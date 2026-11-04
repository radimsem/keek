use super::{ProjectError, ProjectKey};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
pub const INSTALL_LIMIT: std::time::Duration = std::time::Duration::from_secs(20 * 60);
pub type InstallFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<i32>> + Send + 'a>>;
pub trait Installer: Send + Sync {
    fn install<'a>(&'a self, upper: &'a Path, work: &'a Path, log: &'a Path) -> InstallFuture<'a>;
}
enum BuildState {
    Active,
    Finished,
}
struct BuildGuard {
    lock: Option<std::fs::File>,
    build: PathBuf,
    cleanup: fn(&Path) -> std::io::Result<()>,
    runtime: tokio::runtime::Handle,
    state: BuildState,
}
impl Drop for BuildGuard {
    fn drop(&mut self) {
        if matches!(self.state, BuildState::Active)
            && let Some(lock) = self.lock.take()
        {
            let build = self.build.clone();
            let cleanup = self.cleanup;
            // Cleanup must finish before the lock is released.
            self.runtime.spawn_blocking(move || {
                if let Err(error) = cleanup(&build) {
                    tracing::error!(?build, %error, "could not clean the cancelled Project Layer build");
                }
                drop(lock);
            });
        }
    }
}
pub async fn ensure_layer(
    layers: &Path,
    key: &ProjectKey,
    installer: &dyn Installer,
    log: &Path,
) -> Result<PathBuf, ProjectError> {
    ensure_layer_with_cleanup(layers, key, installer, log, remove_build).await
}
async fn ensure_layer_with_cleanup(
    layers: &Path,
    key: &ProjectKey,
    installer: &dyn Installer,
    log: &Path,
    cleanup: fn(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, ProjectError> {
    let layers = layers.to_owned();
    let lock_folder = layers.clone();
    let destination = layers.join(&key.0);
    let check_destination = destination.clone();
    let build = layers.join(".build");
    let guard_build = build.clone();
    let runtime = tokio::runtime::Handle::current();
    let (guard, exists) = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&lock_folder)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_folder.join(".lock"))?;
        lock.lock()?;
        let exists = check_destination.is_dir();
        Ok::<_, std::io::Error>((
            BuildGuard {
                lock: Some(lock),
                build: guard_build,
                cleanup,
                runtime,
                state: if exists {
                    BuildState::Finished
                } else {
                    BuildState::Active
                },
            },
            exists,
        ))
    })
    .await??;
    if exists {
        return Ok(destination);
    }
    let prep = build.clone();
    let guard = tokio::task::spawn_blocking(move || {
        cleanup(&prep)?;
        std::fs::create_dir_all(prep.join("upper"))?;
        std::fs::create_dir_all(prep.join("work"))?;
        Ok::<_, std::io::Error>(guard)
    })
    .await??;
    let result = tokio::time::timeout(
        INSTALL_LIMIT,
        installer.install(&build.join("upper"), &build.join("work"), log),
    )
    .await;
    let outcome = match result {
        Ok(Ok(0)) => Ok(()),
        Ok(Ok(status)) => Err(format!("the install command exited with status {status}")),
        Ok(Err(error)) => Err(format!("the install command could not run: {error}")),
        Err(_) => Err(format!(
            "the install command timed out after {} minutes",
            INSTALL_LIMIT.as_secs() / 60
        )),
    };
    let log = log.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut guard = guard;
        if let Err(mut reason) = outcome {
            if let Err(error) = cleanup(&build) {
                reason.push_str(&format!(". Build cleanup failed: {error}"));
            }
            guard.state = BuildState::Finished;
            let tail = (|| -> std::io::Result<String> {
                let mut file = std::fs::File::open(&log)?;
                let len = file.metadata()?.len();
                file.seek(SeekFrom::Start(len.saturating_sub(4096)))?;
                let mut tail = Vec::new();
                file.read_to_end(&mut tail)?;
                Ok(String::from_utf8_lossy(&tail).into_owned())
            })();
            let log = match tail {
                Ok(tail) => format!("End of {}:\n{tail}", log.display()),
                Err(error) => format!("Cannot read {}: {error}.", log.display()),
            };
            return Err(ProjectError::Install { reason, log });
        }
        std::fs::rename(build.join("upper"), &destination)?;
        if let Err(error) = cleanup(&build) {
            tracing::warn!(path = ?build, %error, "could not clean the Project Layer build");
        }
        guard.state = BuildState::Finished;
        Ok(destination)
    })
    .await?
}
pub(crate) fn remove_build(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    fn unlock(path: &Path) -> std::io::Result<()> {
        let m = std::fs::symlink_metadata(path)?;
        if m.is_dir() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            for entry in std::fs::read_dir(path)? {
                unlock(&entry?.path())?;
            }
        }
        Ok(())
    }
    unlock(path)?;
    std::fs::remove_dir_all(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test]
    async fn cancelled_install_removes_the_build_before_releasing_the_lock() {
        struct Never(tokio::sync::Notify);
        impl Installer for Never {
            fn install<'a>(
                &'a self,
                _: &'a Path,
                work: &'a Path,
                _: &'a Path,
            ) -> InstallFuture<'a> {
                Box::pin(async move {
                    let nested = work.join("work");
                    std::fs::create_dir(&nested)?;
                    std::fs::set_permissions(nested, std::fs::Permissions::from_mode(0o000))?;
                    self.0.notify_one();
                    std::future::pending().await
                })
            }
        }
        let root = tempfile::tempdir().unwrap();
        let installer = Never(tokio::sync::Notify::new());
        let key = ProjectKey("key".into());
        let log = root.path().join("install.log");
        let mut install = Box::pin(ensure_layer(root.path(), &key, &installer, &log));
        tokio::select! {
            () = installer.0.notified() => {},
            result = &mut install => panic!("unexpected result: {result:?}"),
        }
        drop(install);
        let path = root.path().to_owned();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::task::spawn_blocking(move || {
                let lock = std::fs::OpenOptions::new()
                    .write(true)
                    .open(path.join(".lock"))
                    .unwrap();
                lock.lock().unwrap();
                assert!(!path.join(".build").exists());
            }),
        )
        .await
        .unwrap()
        .unwrap();
    }
    struct Fake {
        calls: AtomicUsize,
        status: i32,
    }
    impl Installer for Fake {
        fn install<'a>(
            &'a self,
            upper: &'a Path,
            _work: &'a Path,
            log: &'a Path,
        ) -> InstallFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                std::fs::write(upper.join("installed"), "yes")?;
                std::fs::write(log, format!("head{}tail", "x".repeat(5000)))?;
                Ok(self.status)
            })
        }
    }
    struct MissingLog(i32);
    impl Installer for MissingLog {
        fn install<'a>(&'a self, _: &'a Path, _: &'a Path, _: &'a Path) -> InstallFuture<'a> {
            Box::pin(async move { Ok(self.0) })
        }
    }
    fn fail_existing_cleanup(path: &Path) -> std::io::Result<()> {
        if path.exists() {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fixture cleanup failure",
            ))
        } else {
            Ok(())
        }
    }
    #[tokio::test]
    async fn failed_install_keeps_status_when_cleanup_and_log_fail() {
        let root = tempfile::tempdir().unwrap();
        let error = ensure_layer_with_cleanup(
            root.path(),
            &ProjectKey("key".into()),
            &MissingLog(7),
            &root.path().join("missing.log"),
            fail_existing_cleanup,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("status 7"), "{error}");
        assert!(error.contains("cleanup failure"), "{error}");
        assert!(error.contains("missing.log"), "{error}");
    }
    #[tokio::test]
    async fn successful_install_returns_layer_when_cleanup_fails() {
        let root = tempfile::tempdir().unwrap();
        let result = ensure_layer_with_cleanup(
            root.path(),
            &ProjectKey("key".into()),
            &MissingLog(0),
            &root.path().join("missing.log"),
            fail_existing_cleanup,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
        assert!(result.unwrap().is_dir());
    }
    #[tokio::test]
    async fn project_layer_is_reused_for_the_same_key() {
        let d = tempfile::tempdir().unwrap();
        let f = Fake {
            calls: AtomicUsize::new(0),
            status: 0,
        };
        let log = d.path().join("install.log");
        let key = ProjectKey("a".into());
        let layer = ensure_layer(d.path(), &key, &f, &log).await.unwrap();
        assert!(layer.join("installed").exists());
        ensure_layer(d.path(), &key, &f, &log).await.unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        assert!(!d.path().join(".build").exists());
    }
    #[tokio::test]
    async fn project_layer_is_rebuilt_for_a_changed_key() {
        let d = tempfile::tempdir().unwrap();
        let f = Fake {
            calls: AtomicUsize::new(0),
            status: 0,
        };
        let log = d.path().join("install.log");
        ensure_layer(d.path(), &ProjectKey("a".into()), &f, &log)
            .await
            .unwrap();
        ensure_layer(d.path(), &ProjectKey("b".into()), &f, &log)
            .await
            .unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn project_failed_install_quotes_only_the_log_tail() {
        let d = tempfile::tempdir().unwrap();
        let f = Fake {
            calls: AtomicUsize::new(0),
            status: 7,
        };
        let log = d.path().join("install.log");
        let e = ensure_layer(d.path(), &ProjectKey("key".into()), &f, &log)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("tail") && !e.contains("head"), "{e}");
        assert!(
            e.contains("status 7") && e.contains(log.to_str().unwrap()),
            "{e}"
        );
    }
    #[tokio::test]
    async fn project_failed_install_ends_its_message_with_the_log() {
        let d = tempfile::tempdir().unwrap();
        let f = Fake {
            calls: AtomicUsize::new(0),
            status: 7,
        };
        let e = ensure_layer(
            d.path(),
            &ProjectKey("key".into()),
            &f,
            &d.path().join("install.log"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.ends_with("tail"), "{e}");
    }
    #[tokio::test]
    async fn project_install_that_cannot_start_names_the_failure_once() {
        struct CannotStart;
        impl Installer for CannotStart {
            fn install<'a>(&'a self, _: &'a Path, _: &'a Path, _: &'a Path) -> InstallFuture<'a> {
                Box::pin(async { Err(std::io::Error::other("fixture spawn failure")) })
            }
        }
        let d = tempfile::tempdir().unwrap();
        let e = ensure_layer(
            d.path(),
            &ProjectKey("key".into()),
            &CannotStart,
            &d.path().join("install.log"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("fixture spawn failure"), "{e}");
        assert_eq!(e.matches("Cannot prepare the Project").count(), 0, "{e}");
    }
    #[tokio::test]
    async fn project_failed_install_removes_its_build() {
        let d = tempfile::tempdir().unwrap();
        let f = Fake {
            calls: AtomicUsize::new(0),
            status: 7,
        };
        let _ = ensure_layer(
            d.path(),
            &ProjectKey("key".into()),
            &f,
            &d.path().join("install.log"),
        )
        .await
        .unwrap_err();
        assert!(!d.path().join("key").exists());
        assert!(!d.path().join(".build").exists());
    }
    #[tokio::test]
    async fn project_layer_cleans_a_stale_mode_zero_work_directory() {
        let d = tempfile::tempdir().unwrap();
        let stale = d.path().join(".build/work/work");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o0)).unwrap();
        let f = Fake {
            calls: AtomicUsize::new(0),
            status: 0,
        };
        ensure_layer(
            d.path(),
            &ProjectKey("key".into()),
            &f,
            &d.path().join("log"),
        )
        .await
        .unwrap();
        assert!(!d.path().join(".build").exists());
    }
    #[tokio::test]
    async fn project_concurrent_opens_install_once() {
        let d = tempfile::tempdir().unwrap();
        let f = Fake {
            calls: AtomicUsize::new(0),
            status: 0,
        };
        let key = ProjectKey("key".into());
        let log = d.path().join("log");
        let (a, b) = tokio::join!(
            ensure_layer(d.path(), &key, &f, &log),
            ensure_layer(d.path(), &key, &f, &log)
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn project_install_timeout_cancels_install_and_cleans_the_build() {
        use std::sync::atomic::AtomicBool;
        struct Pending(AtomicBool);
        struct Cancelled<'a>(&'a AtomicBool);
        impl Drop for Cancelled<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        impl Installer for Pending {
            fn install<'a>(
                &'a self,
                _upper: &'a Path,
                _work: &'a Path,
                log: &'a Path,
            ) -> InstallFuture<'a> {
                Box::pin(async move {
                    let _cancelled = Cancelled(&self.0);
                    std::fs::write(log, "install still running")?;
                    std::future::pending::<()>().await;
                    Ok(0)
                })
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let installer = Pending(AtomicBool::new(false));
        let error = ensure_layer(
            directory.path(),
            &ProjectKey("key".into()),
            &installer,
            &directory.path().join("install.log"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(installer.0.load(Ordering::SeqCst));
        assert!(
            error.contains("20 minutes") && error.contains("install still running"),
            "{error}"
        );
        assert!(!directory.path().join(".build").exists());
        assert!(!directory.path().join("key").exists());
    }
}
