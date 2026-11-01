use std::env;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use keek_sandbox_helper::{list_globals, open_security_context};
use wayland_client::Connection;

const ENGINE: &str = "dev.keek";
const APP_ID: &str = "keek-session";
// The sandbox sets WAYLAND_SOCKET to this number for Hyprland alone.
const CHILD_WAYLAND_FD: i32 = 3;

const USAGE: &str = "usage:
  keek-sandbox-helper globals --host              list globals on a plain host connection
  keek-sandbox-helper globals --security-context  list globals through a security context
  keek-sandbox-helper connect-exec -- CMD [ARG]   run CMD with that connection on fd 3";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match refs.as_slice() {
        ["globals", "--host"] => print_host_globals(),
        ["globals", "--security-context"] => print_sandboxed_globals(),
        ["connect-exec", "--", command, rest @ ..] => connect_exec(command, rest),
        _ => Err(USAGE.to_owned()),
    };
    result.unwrap_or_else(|message| {
        eprintln!("{message}");
        ExitCode::FAILURE
    })
}

fn runtime_dir() -> Result<PathBuf, String> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| {
            "XDG_RUNTIME_DIR isn't set, so there is nowhere to put the listener socket. Run this from a desktop session.".to_owned()
        })
}

fn print_host_globals() -> Result<ExitCode, String> {
    let conn = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let lines = list_globals(&conn).map_err(|error| error.to_string())?;
    println!("{}", lines.join("\n"));
    Ok(ExitCode::SUCCESS)
}

fn print_sandboxed_globals() -> Result<ExitCode, String> {
    let opened = open_security_context(&runtime_dir()?, ENGINE, APP_ID)
        .map_err(|error| error.to_string())?;
    let conn = Connection::from_socket(opened.stream).map_err(|error| error.to_string())?;
    let lines = list_globals(&conn).map_err(|error| error.to_string())?;
    println!("{}", lines.join("\n"));
    drop(opened.listener_guard);
    Ok(ExitCode::SUCCESS)
}

fn connect_exec(command: &str, args: &[&str]) -> Result<ExitCode, String> {
    die_with_parent()?;
    let opened = open_security_context(&runtime_dir()?, ENGINE, APP_ID)
        .map_err(|error| error.to_string())?;
    let source_fd = opened.stream.as_raw_fd();

    let mut child = Command::new(command);
    child.args(args);
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
    let mut running = child
        .spawn()
        .map_err(|error| format!("Can't start {command}: {error}. Check that it is installed."))?;
    drop(opened.stream);

    // The guard lives until the child exits, so the listener outlasts the
    // moment the nested compositor is accepted.
    let status = running.wait().map_err(|error| error.to_string())?;
    drop(opened.listener_guard);
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn die_with_parent() -> Result<(), String> {
    // SAFETY: PR_SET_PDEATHSIG takes the signal number and reads no pointers.
    let result = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) };
    if result == -1 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
