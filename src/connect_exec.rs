use crate::security_context::open_security_context;
use std::env;
use std::ffi::OsStr;
use std::io::{Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
const ENGINE: &str = "dev.keek";
const APP_ID: &str = "keek-session";
const CHILD_WAYLAND_FD: i32 = 3;
// Reserved before any other descriptor opens, so this is the only number bwrap can name.
pub(crate) const SANDBOX_ARGS_FD: i32 = 4;
#[derive(Debug, thiserror::Error)]
pub enum ConnectExecError {
    #[error(
        "XDG_RUNTIME_DIR isn't set, so there is nowhere to put the listener socket. Run this from a desktop session."
    )]
    NoRuntimeDir,
    #[error(transparent)]
    LanBlock(#[from] crate::lan_block::LanBlockError),
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
    #[error(
        "Can't prepare the sandbox's arguments file: {0}. Check that memfd_create is available and retry."
    )]
    ArgumentsFile(std::io::Error),
    #[error(
        "Can't read the Test Credentials handed to connect-exec: they aren't valid text. Restart the Agent Session."
    )]
    SandboxChannel,
}
fn runtime_dir() -> Result<PathBuf, ConnectExecError> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or(ConnectExecError::NoRuntimeDir)
}
pub fn run(command: &str, args: &[String]) -> Result<ExitCode, ConnectExecError> {
    die_with_parent()?;
    run_with(&crate::lan_block::HostIp, command, args)
}

fn run_with(
    tool: &dyn crate::lan_block::IpTool,
    command: &str,
    args: &[String],
) -> Result<ExitCode, ConnectExecError> {
    crate::lan_block::block_lan(tool)?;
    let arguments = reserve_args_file(&sandbox_arguments(
        env::var_os(crate::credentials::SANDBOX_ENV).as_deref(),
    )?)?;
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
    // bwrap already holds its own copy from exec, so nothing here keeps the values open.
    drop(arguments);

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
fn sandbox_arguments(channel: Option<&OsStr>) -> Result<Vec<u8>, ConnectExecError> {
    let text = match channel {
        None => "",
        Some(raw) => raw.to_str().ok_or(ConnectExecError::SandboxChannel)?,
    };
    let mut data = Vec::new();
    for (name, value) in crate::credentials::Credentials::from_sandbox_env(text) {
        data.extend_from_slice(b"--setenv\0");
        data.extend_from_slice(name.as_bytes());
        data.push(0);
        data.extend_from_slice(value.as_bytes());
        data.push(0);
    }
    Ok(data)
}

fn reserve_args_file(data: &[u8]) -> Result<OwnedFd, ConnectExecError> {
    // SAFETY: memfd_create copies the name and reads no other pointer.
    let created = unsafe { libc::syscall(libc::SYS_memfd_create, c"keek-args", libc::MFD_CLOEXEC) };
    if created < 0 {
        return Err(ConnectExecError::ArgumentsFile(
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: the number is fresh and owned nowhere else.
    let mut file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(created as i32) });
    file.write_all(data)
        .map_err(ConnectExecError::ArgumentsFile)?;
    // bwrap reads from the current offset, so the file must point at its start again.
    file.rewind().map_err(ConnectExecError::ArgumentsFile)?;
    if file.as_raw_fd() == SANDBOX_ARGS_FD {
        // SAFETY: F_SETFD takes only a flag word.
        if unsafe { libc::fcntl(SANDBOX_ARGS_FD, libc::F_SETFD, 0) } == -1 {
            return Err(ConnectExecError::ArgumentsFile(
                std::io::Error::last_os_error(),
            ));
        }
        return Ok(OwnedFd::from(file));
    }
    // SAFETY: dup2 takes two integers and shares no memory.
    if unsafe { libc::dup2(file.as_raw_fd(), SANDBOX_ARGS_FD) } == -1 {
        return Err(ConnectExecError::ArgumentsFile(
            std::io::Error::last_os_error(),
        ));
    }
    // `file` drops here and closes the original; fd 4 keeps its own copy, open across exec.
    // SAFETY: fd 4 is ours now and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(SANDBOX_ARGS_FD) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connect_exec_does_not_start_the_command_when_the_lan_cannot_be_blocked() {
        struct FailingTool;
        impl crate::lan_block::IpTool for FailingTool {
            fn run(&self, args: &[&str]) -> Result<(), crate::lan_block::LanBlockError> {
                Err(crate::lan_block::LanBlockError::Rule {
                    command: format!("ip {}", args.join(" ")),
                    detail: "Operation not permitted".into(),
                })
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("marker");
        let args = ["-c".into(), format!("touch {}", marker.display())];
        let result = run_with(&FailingTool, "sh", &args);
        assert!(
            matches!(result, Err(ConnectExecError::LanBlock(_))),
            "{result:?}"
        );
        assert!(!marker.exists());
    }

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

    fn arguments(channel: &str) -> Vec<u8> {
        sandbox_arguments(Some(OsStr::new(channel))).unwrap()
    }

    #[test]
    fn credential_values_become_setenv_arguments() {
        assert_eq!(
            arguments("PASSWORD=pa\"ss\\\\w&rd 1=x\nOTHER=long enough"),
            b"--setenv\0PASSWORD\0pa\"ss\\\\w&rd 1=x\0--setenv\0OTHER\0long enough\0"
        );
    }

    #[test]
    fn an_absent_or_malformed_channel_gives_no_arguments() {
        assert!(sandbox_arguments(None).unwrap().is_empty());
        assert!(arguments("no-separator").is_empty());
        assert!(arguments("bad name=value").is_empty());
    }

    #[test]
    fn a_channel_that_isnt_text_refuses_the_session() {
        use std::os::unix::ffi::OsStringExt;
        let raw = std::ffi::OsString::from_vec(vec![0xff, b'=', b'x']);
        assert!(matches!(
            sandbox_arguments(Some(&raw)),
            Err(ConnectExecError::SandboxChannel)
        ));
    }
}
