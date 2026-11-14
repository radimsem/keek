use crate::project::{InstallFuture, Installer, ProjectFiles};
use crate::sandbox::SandboxSpec;
use crate::session::{Mounts, Opening, SessionError};
use crate::session_factory::SandboxFactory;
use crate::session_folder::SessionFolder;
use std::path::{Path, PathBuf};

pub fn tree_path(trees: &Path, name: &crate::session_folder::SessionName) -> PathBuf {
    trees.join(name.to_string())
}

pub async fn spec(
    factory: &SandboxFactory,
    folder: &SessionFolder,
    opening: &Opening,
) -> Result<SandboxSpec, SessionError> {
    let root = folder.root().to_owned();
    let driver = factory.driver.clone();
    let project = opening.project.clone();
    let worktree = opening.worktree.clone();
    let credentials = opening.credentials.clone();
    let test_files = opening.test_files.clone();
    let profile = opening.profile.clone();
    tokio::task::spawn_blocking(move || {
        let mut spec = SandboxSpec::from_env(&root, &driver)?;
        spec.credentials = credentials;
        spec.test_files = test_files;
        spec.project = project;
        spec.worktree = worktree;
        spec.profile = profile;
        Ok(spec)
    })
    .await
    .map_err(|e| SessionError::Refused(format!("Cannot prepare the Project: {e}. Open again.")))?
}

pub async fn prepare(
    factory: &SandboxFactory,
    folder: &SessionFolder,
    opening: &Opening,
) -> Result<Mounts, SessionError> {
    let mut spec = spec(factory, folder, opening).await?;
    if !spec.profile.toolchain.is_empty() {
        opening.progress.send("Installing toolchains");
    }
    spec.toolchains = crate::toolchain::resolve(
        factory.mise.as_ref(),
        &factory.toolchains,
        &spec.profile.toolchain,
        &folder.root().join("toolchain.log"),
    )
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))?;
    let files = ProjectFiles::read(factory.runner.as_ref(), &spec.worktree)
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))?;
    let worktree = spec.worktree.clone();
    let (spec_profile, toolchains) = (spec.profile.clone(), spec.toolchains.clone());
    let tree = tree_path(&factory.trees, &opening.name);
    // The tree stays outside the writable session folder so the sandbox cannot change an overlay lower layer.
    let build_tree = tree.clone();
    let key = tokio::task::spawn_blocking(move || {
        let key = files
            .key(&worktree, &spec_profile, &toolchains)
            .map_err(|e| SessionError::Refused(e.to_string()))?;
        files.build_tree(&worktree, &build_tree).map_err(|e| {
            SessionError::Refused(format!(
                "Cannot copy the worktree's files: {e}. Check the worktree and open again."
            ))
        })?;
        Ok::<_, SessionError>(key)
    })
    .await
    .map_err(|e| {
        SessionError::Refused(format!(
            "Cannot copy the worktree's files: {e}. Check the worktree and open again."
        ))
    })?
    .map_err(|e| SessionError::Refused(e.to_string()))?;
    spec.tree = tree.clone();
    let prep = spec.clone();
    tokio::task::spawn_blocking(move || prep.prepare())
        .await
        .map_err(|e| {
            SessionError::Refused(format!(
                "Cannot prepare the Agent Session files: {e}. Open again."
            ))
        })??;
    let layer = crate::project::ensure_layer(
        &factory.projects_cache.join(&spec.project),
        &key,
        &Install(&spec, &opening.progress),
        &folder.root().join("install.log"),
    )
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))?;
    Ok(Mounts {
        tree,
        layer: layer.path,
        lease: Some(layer.lease),
        toolchains: spec.toolchains,
    })
}
pub(crate) struct Install<'a>(
    pub(crate) &'a SandboxSpec,
    pub(crate) &'a crate::open_progress::OpenProgress,
);
impl Installer for Install<'_> {
    fn install<'a>(&'a self, upper: &'a Path, work: &'a Path, log: &'a Path) -> InstallFuture<'a> {
        Box::pin(async move {
            if self.0.profile.install.trim().is_empty() {
                let log = log.to_owned();
                tokio::task::spawn_blocking(move || {
                    std::fs::write(log, "The Project Profile has no install command.\n")
                })
                .await??;
                return Ok(0);
            }
            let spec = self.0.clone();
            let upper = upper.to_owned();
            let work = work.to_owned();
            let command_log = log.to_owned();
            let mut command = tokio::task::spawn_blocking(move || {
                spec.install_command(&upper, &work, &command_log)
            })
            .await?
            .map_err(std::io::Error::other)?;
            let mut child = command.spawn()?;
            let status = self.1.tailing(log, child.wait()).await?;
            Ok(status.code().unwrap_or(-1))
        })
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    use std::sync::Arc;

    struct Runner;
    impl crate::process::ProcessRunner for Runner {
        fn output<'a>(
            &'a self,
            _: &'a str,
            _: &'a [&'a str],
            _: Option<&'a Path>,
        ) -> crate::process::ProcessFuture<'a> {
            Box::pin(async { Ok(String::new()) })
        }
    }
    struct Fake;
    impl Installer for Fake {
        fn install<'a>(&'a self, _: &'a Path, _: &'a Path, _: &'a Path) -> InstallFuture<'a> {
            Box::pin(async { Ok(0) })
        }
    }
    #[tokio::test]
    async fn prepared_mounts_keep_the_project_layer_until_dropped() {
        let root = tempfile::tempdir().unwrap();
        let runner = Arc::new(Runner);
        let factory = SandboxFactory {
            driver: root.path().join("driver"),
            runner: runner.clone(),
            compositor: Arc::new(crate::compositor::HostCompositor::new(runner)),
            projects_cache: root.path().join("projects"),
            trees: root.path().join("trees"),
            toolchains: root.path().join("toolchains"),
            mise: Arc::new(crate::toolchain::HostMise),
            prepare: root.path().join("prepare"),
        };
        let name = "app_12-00-00".parse().unwrap();
        let folder = SessionFolder::new(root.path(), &name);
        let opening = Opening {
            test_files: None,
            placement: crate::compositor::Placement {
                geometry: crate::slots::geometry(
                    crate::slots::Slot {
                        x: 0,
                        y: 0,
                        width: 1280,
                        height: 800,
                    },
                    None,
                ),
                monitor: String::new(),
                origin: (0, 0),
            },
            credentials: Default::default(),
            project: "app".into(),
            worktree: root.path().to_owned(),
            profile: Default::default(),
            name,
            progress: Default::default(),
        };
        let mounts = prepare(&factory, &folder, &opening).await.unwrap();
        let layer = mounts.layer.clone();
        let layers = factory.projects_cache.join("app");
        for key in ["b", "c"] {
            drop(
                crate::project::ensure_layer(
                    &layers,
                    &crate::project::ProjectKey(key.into()),
                    &Fake,
                    &root.path().join("log"),
                )
                .await
                .unwrap(),
            );
        }
        assert!(layer.is_dir());
        drop(mounts);
        drop(
            crate::project::ensure_layer(
                &layers,
                &crate::project::ProjectKey("d".into()),
                &Fake,
                &root.path().join("log"),
            )
            .await
            .unwrap(),
        );
        assert!(!layer.exists());
    }
}
