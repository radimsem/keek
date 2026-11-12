//! `prepare_project`: build or reuse the Project Layer and run `start` briefly, with no desktop or recording.

use crate::credentials::Credentials;
use crate::ipc::{LayerUse, PreparedProject, PreparedTool, StartProbe};
use crate::session::SessionError;
use crate::session_factory::SandboxFactory;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long `start` runs before keek reports it as running and stops it.
pub const START_PROBE: Duration = Duration::from_secs(15);
const TAIL_BYTES: u64 = 4096;

pub struct ProjectPrep {
    pub test_files: Option<PathBuf>,
    pub project: String,
    pub worktree: PathBuf,
    pub profile: crate::project::Profile,
    pub credentials: Credentials,
    pub progress: crate::open_progress::OpenProgress,
}

pub type PrepareFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<PreparedProject, SessionError>> + Send + 'a>,
>;

/// Waits up to `limit` for `child` to exit, and kills it if it is still running then.
pub async fn probe(
    child: &mut tokio::process::Child,
    limit: Duration,
) -> std::io::Result<StartProbe> {
    match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => Ok(StartProbe::Exited {
            status: status?.code().unwrap_or(-1),
        }),
        Err(_) => {
            child.kill().await?;
            Ok(StartProbe::Running)
        }
    }
}

/// The last 4 KiB of `log`, masked. A missing log is empty.
pub fn masked_tail(log: &Path, credentials: &Credentials) -> String {
    // A value cut by the start of the tail would escape masking, so read one longest value more and drop it after masking.
    let margin = credentials.longest_form();
    let read = || -> std::io::Result<(String, bool)> {
        let mut file = std::fs::File::open(log)?;
        let start = file
            .metadata()?
            .len()
            .saturating_sub(TAIL_BYTES + margin as u64);
        file.seek(SeekFrom::Start(start))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        Ok((String::from_utf8_lossy(&tail).into_owned(), start > 0))
    };
    match read() {
        Ok((tail, cut)) => {
            let masked = credentials.mask(&tail);
            if !cut {
                return masked;
            }
            let from = (margin..=masked.len())
                .find(|&index| masked.is_char_boundary(index))
                .unwrap_or(masked.len());
            masked[from..].to_owned()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => format!("Cannot read {}: {error}.", log.display()),
    }
}

fn refused(what: &str) -> impl Fn(std::io::Error) -> SessionError + '_ {
    move |error| {
        SessionError::Refused(format!(
            "Cannot {what}: {error}. Check the folder and call prepare_project again."
        ))
    }
}

async fn blocking<T: Send + 'static>(
    what: &str,
    work: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> Result<T, SessionError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(std::io::Error::other)
        .and_then(|result| result)
        .map_err(refused(what))
}

pub async fn prepare(
    factory: &SandboxFactory,
    prep: &ProjectPrep,
) -> Result<PreparedProject, SessionError> {
    let folder = factory.prepare.join(&prep.project);
    let fresh = folder.clone();
    blocking("clear the prepare folder", move || {
        match std::fs::remove_dir_all(&fresh) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        std::fs::create_dir_all(&fresh)
    })
    .await?;
    prep.progress.send("Building Project Layer");
    if !prep.profile.toolchain.is_empty() {
        prep.progress.send("Installing toolchains");
    }
    let toolchains = crate::toolchain::resolve(
        factory.mise.as_ref(),
        &factory.toolchains,
        &prep.profile.toolchain,
        &folder.join("toolchain.log"),
    )
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))?;
    let files = crate::project::ProjectFiles::read(factory.runner.as_ref(), &prep.worktree)
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))?;
    let tree = folder.join("tree");
    let (worktree, profile, build_tree, key_toolchains) = (
        prep.worktree.clone(),
        prep.profile.clone(),
        tree.clone(),
        toolchains.clone(),
    );
    let key = tokio::task::spawn_blocking(move || {
        let key = files
            .key(&worktree, &profile, &key_toolchains)
            .map_err(|e| SessionError::Refused(e.to_string()))?;
        files.build_tree(&worktree, &build_tree).map_err(|e| {
            SessionError::Refused(format!(
                "Cannot copy the worktree's files: {e}. Check the worktree and call prepare_project again."
            ))
        })?;
        Ok::<_, SessionError>(key)
    })
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))??;
    let (root, driver) = (folder.clone(), factory.driver.clone());
    let mut spec = blocking("prepare the sandbox files", move || {
        let spec =
            crate::sandbox::SandboxSpec::from_env(&root, &driver).map_err(std::io::Error::other)?;
        spec.prepare().map_err(std::io::Error::other)?;
        Ok(spec)
    })
    .await?;
    spec.credentials = prep.credentials.clone();
    spec.project = prep.project.clone();
    spec.worktree = prep.worktree.clone();
    spec.profile = prep.profile.clone();
    spec.tree = tree.clone();
    spec.toolchains = toolchains;
    let install_log = folder.join("install.log");
    let prepared_layer = crate::project::ensure_layer(
        &factory.projects_cache.join(&spec.project),
        &key,
        &crate::session_project::Install(&spec, &prep.progress),
        &install_log,
    )
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))?;
    spec.layer = prepared_layer.path;
    let lease = prepared_layer.lease;
    // The folder starts empty, so only a build writes install.log.
    let layer = if install_log.exists() {
        LayerUse::Built
    } else {
        LayerUse::Reused
    };
    prep.progress.send("Starting App Under Test");
    let start_log = folder.join("start.log");
    spec.test_files = prep.test_files.clone();
    let probe_spec = spec.clone();
    let probe_log = start_log.clone();
    let mut command = blocking("prepare the start command", move || {
        probe_spec
            .probe_command(&probe_log)
            .map_err(std::io::Error::other)
    })
    .await?;
    // PDEATHSIG follows the spawning thread, so spawn on the runtime, never in spawn_blocking.
    let mut child = command.spawn().map_err(SessionError::SpawnPasta)?;
    let start = probe(&mut child, START_PROBE)
        .await
        .map_err(refused("run the start command"))?;
    let credentials = prep.credentials.clone();
    let tails = blocking("read the logs", move || {
        let tails = (
            masked_tail(&install_log, &credentials),
            masked_tail(&start_log, &credentials),
        );
        crate::project::layer::remove_build(&tree)?;
        Ok(tails)
    })
    .await?;
    drop(lease);
    Ok(PreparedProject {
        project: prep.project.clone(),
        worktree: prep.worktree.clone(),
        toolchains: spec
            .toolchains
            .tools
            .iter()
            .map(|tool| PreparedTool {
                tool: tool.tool,
                version: tool.version.clone(),
                fetch: tool.fetch,
            })
            .collect(),
        layer,
        install_log: tails.0,
        start,
        start_log: tails.1,
        warnings: vec![],
    })
}

#[cfg(test)]
#[path = "project_prepare_tests.rs"]
mod tests;
