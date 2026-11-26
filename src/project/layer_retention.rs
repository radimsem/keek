use super::ProjectKey;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const KEPT_LAYERS: usize = 2;

// A shared lock on .used/<key> marks a layer as mounted.
#[derive(Debug)]
pub struct LayerLease {
    file: File,
}

impl Drop for LayerLease {
    fn drop(&mut self) {
        // A child forked meanwhile shares this lock until it execs, so closing alone keeps it.
        if let Err(error) = self.file.unlock() {
            tracing::warn!(%error, "could not release a Project Layer lease");
        }
    }
}

#[derive(Debug)]
pub struct Layer {
    pub path: PathBuf,
    pub lease: LayerLease,
}

pub(super) fn lease(layers: &Path, key: &ProjectKey) -> io::Result<LayerLease> {
    // A layer may be a mounted overlay lower layer, so record use in a sidecar.
    let used = layers.join(".used");
    std::fs::create_dir_all(&used)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(used.join(&key.0))?;
    file.lock_shared()?;
    file.set_modified(SystemTime::now())?;
    Ok(LayerLease { file })
}

// The caller holds the Project lock, under which leases are taken.
pub(super) fn prune(layers: &Path, keep: usize) {
    let candidates = (|| -> io::Result<Vec<_>> {
        let mut candidates = Vec::new();
        for entry in std::fs::read_dir(layers)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.as_encoded_bytes().starts_with(b".removing-") && entry.file_type()?.is_dir() {
                // One leftover that can't be removed must not stop pruning the rest.
                if let Err(error) = super::layer::remove_build(&entry.path()) {
                    let path = entry.path();
                    tracing::warn!(?path, %error, "could not remove old Project Layer");
                }
                continue;
            }
            if name.as_encoded_bytes().starts_with(b".") || !entry.file_type()?.is_dir() {
                continue;
            }
            let sidecar = layers.join(".used").join(&name);
            let modified = match std::fs::metadata(&sidecar) {
                Ok(metadata) => metadata.modified()?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => UNIX_EPOCH,
                Err(error) => return Err(error),
            };
            candidates.push((modified, name));
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        Ok(candidates)
    })();
    let candidates = match candidates {
        Ok(candidates) => candidates,
        Err(error) => {
            tracing::warn!(?layers, %error, "could not list old Project Layers");
            return;
        }
    };
    for (_, name) in candidates.into_iter().skip(keep) {
        let path = layers.join(&name);
        let sidecar = layers.join(".used").join(&name);
        let result = (|| -> io::Result<()> {
            let file = match OpenOptions::new().write(true).open(&sidecar) {
                Ok(file) => Some(file),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            if let Some(file) = &file {
                match file.try_lock() {
                    Ok(()) => {}
                    Err(TryLockError::WouldBlock) => return Ok(()),
                    Err(TryLockError::Error(error)) => return Err(error),
                }
            }
            let mut removing = std::ffi::OsString::from(".removing-");
            removing.push(&name);
            let removing = layers.join(removing);
            std::fs::rename(&path, &removing)?;
            if file.is_some() {
                std::fs::remove_file(&sidecar)?;
            }
            super::layer::remove_build(&removing)?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(?path, %error, "could not remove old Project Layer");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Layer;
    use crate::project::{InstallFuture, Installer, ProjectKey, ensure_layer};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    struct Fake;

    impl Installer for Fake {
        fn install<'a>(&'a self, upper: &'a Path, _: &'a Path, _: &'a Path) -> InstallFuture<'a> {
            Box::pin(async move {
                std::fs::write(upper.join("installed"), "yes")?;
                Ok(0)
            })
        }
    }

    async fn ensure(layers: &Path, key: &str) -> Layer {
        ensure_layer(
            layers,
            &ProjectKey(key.into()),
            &Fake,
            &layers.join("install.log"),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_layer_left_half_removed_is_cleared_and_never_reused() {
        let root = tempfile::tempdir().unwrap();
        let layers = root.path();
        std::fs::create_dir(layers.join(".removing-a")).unwrap();
        std::fs::write(layers.join(".removing-a/partial"), "old").unwrap();
        let layer = ensure(layers, "a").await;
        assert!(layer.path.join("installed").exists());
        assert!(!layers.join(".removing-a").exists());
    }

    #[tokio::test]
    async fn a_third_project_layer_removes_the_least_recently_used_one() {
        let root = tempfile::tempdir().unwrap();
        let layers = root.path();
        for key in ["a", "b", "c"] {
            drop(ensure(layers, key).await);
        }
        assert!(!layers.join("a").exists());
        assert!(layers.join("b/installed").exists());
        assert!(layers.join("c/installed").exists());
    }

    #[tokio::test]
    async fn reusing_a_project_layer_makes_it_the_most_recently_used() {
        let root = tempfile::tempdir().unwrap();
        let layers = root.path();
        for key in ["a", "b", "a", "c"] {
            drop(ensure(layers, key).await);
        }
        assert!(!layers.join("b").exists());
        assert!(layers.join("a/installed").exists());
        assert!(layers.join("c/installed").exists());
    }

    #[tokio::test]
    async fn a_project_layer_an_open_session_mounts_is_never_removed() {
        let root = tempfile::tempdir().unwrap();
        let layers = root.path();
        let mounted = ensure(layers, "a").await;
        for key in ["b", "c", "d"] {
            drop(ensure(layers, key).await);
        }
        assert!(layers.join("a/installed").exists());
        assert!(!layers.join("b").exists());
        assert!(layers.join("c/installed").exists());
        assert!(layers.join("d/installed").exists());
        drop(mounted);
        drop(ensure(layers, "e").await);
        assert!(!layers.join("a").exists());
    }

    #[tokio::test]
    async fn a_closed_lease_frees_its_layer_while_a_forked_child_holds_a_copy() {
        let root = tempfile::tempdir().unwrap();
        let layers = root.path();
        let mounted = ensure(layers, "a").await;
        let child_copy = mounted.lease.file.try_clone().unwrap();
        for key in ["b", "c"] {
            drop(ensure(layers, key).await);
        }
        drop(mounted);
        drop(ensure(layers, "d").await);
        assert!(!layers.join("a").exists());
        drop(child_copy);
    }

    #[tokio::test]
    async fn a_project_layer_with_no_recorded_use_is_removed_first() {
        let root = tempfile::tempdir().unwrap();
        let layers = root.path();
        std::fs::create_dir(layers.join("old")).unwrap();
        for key in ["a", "b"] {
            drop(ensure(layers, key).await);
        }
        assert!(!layers.join("old").exists());
        assert!(layers.join("a/installed").exists());
        assert!(layers.join("b/installed").exists());
    }

    #[tokio::test]
    async fn pruning_removes_a_layer_with_read_only_folders() {
        let root = tempfile::tempdir().unwrap();
        let layers = root.path();
        drop(ensure(layers, "a").await);
        let readonly = layers.join("a/readonly");
        std::fs::create_dir(&readonly).unwrap();
        std::fs::write(readonly.join("module"), "cached").unwrap();
        std::fs::set_permissions(readonly, std::fs::Permissions::from_mode(0o500)).unwrap();
        for key in ["b", "c"] {
            drop(ensure(layers, key).await);
        }
        assert!(!layers.join("a").exists());
        assert!(layers.join("b/installed").exists());
        assert!(layers.join("c/installed").exists());
    }
}
