use crate::security_context::open_security_context;
use std::env;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
const ENGINE: &str = "dev.keek";
const APP_ID: &str = "keek-session";
const CHILD_WAYLAND_FD: i32 = 3;
#[derive(Debug, thiserror::Error)]
pub enum ConnectExecError {
    #[error(
        "XDG_RUNTIME_DIR isn't set, so there is nowhere to put the listener socket. Run this from a desktop session."
    )]
    NoRuntimeDir,
    #[error(transparent)]
    SecurityContext(#[from] crate::security_context::Error),
    #[error("Can't start {command}: {source}. Check that it is installed.")]
    Spawn {
        command: String,
        source: std::io::Error,
    },
    #[error("Could not wait for the sandbox command: {0}. Check the sandbox logs and retry.")]
    Wait(std::io::Error),
    #[error(
        "Could not arrange for the sandbox to exit with its parent: {0}. Check system support and retry."
    )]
    ParentSignal(std::io::Error),
}
fn runtime_dir() -> Result<PathBuf, ConnectExecError> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or(ConnectExecError::NoRuntimeDir)
}
pub fn run(command: &str, args: &[String]) -> Result<ExitCode, ConnectExecError> {
    die_with_parent()?;
    let opened = open_security_context(&runtime_dir()?, ENGINE, APP_ID)?;
    let source_fd = opened.stream.as_raw_fd();

    let mut child = child_command(command, args);
    // SAFETY: dup2 and fcntl are async-signal-safe, and the closure touches
    // nothing but these two integers.
    unsafe {
        child.pre_exec(move || {
            if source_fd == CHILD_WAYLAND_FD {
                if libc::fcntl(source_fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(source_fd, CHILD_WAYLAND_FD) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut running = child.spawn().map_err(|source| ConnectExecError::Spawn {
        command: command.to_owned(),
        source,
    })?;
    drop(opened.stream);

    // The guard lives until the child exits, so the listener outlasts the
    // moment the nested compositor is accepted.
    let status = running.wait().map_err(ConnectExecError::Wait)?;
    drop(opened.listener_guard);
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn die_with_parent() -> Result<(), ConnectExecError> {
    // SAFETY: PR_SET_PDEATHSIG takes the signal number and reads no pointers.
    let result = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) };
    if result == -1 {
        return Err(ConnectExecError::ParentSignal(
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

fn child_command(command: &str, args: &[String]) -> Command {
    let mut child = Command::new(command);
    child.args(args).env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        child.env("PATH", path);
    }
    child
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_inherits_only_path() {
        let child = child_command("bwrap", &[]);
        assert!(format!("{child:?}").contains("env -i"));
        let envs: Vec<_> = child.get_envs().collect();
        assert_eq!(envs.len(), usize::from(std::env::var_os("PATH").is_some()));
        if let Some(path) = std::env::var_os("PATH") {
            assert_eq!(
                envs[0],
                (std::ffi::OsStr::new("PATH"), Some(path.as_os_str()))
            );
        }
    }
}
