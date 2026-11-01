use std::fs::{self, File};
use std::os::unix::fs::OpenOptionsExt;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use keek::clock::MonotonicClock;
use keek::daemon::{self, Exit, IDLE_LIMIT_ENV};
use keek::paths::Paths;

#[derive(Parser)]
#[command(
    version,
    about = "Verify a change on a sandboxed nested Hyprland desktop"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve MCP over stdio for one coding agent.
    Mcp,
    #[command(hide = true)]
    ConnectExec {
        #[arg(last = true, required = true)]
        args: Vec<String>,
    },
    /// Run the per-user daemon that `keek mcp` starts.
    Daemon,
}

fn main() -> Result<std::process::ExitCode> {
    match Cli::parse().command {
        Command::ConnectExec { args } => {
            let (command, rest) = args
                .split_first()
                .context("Cannot start connect-exec because its command is missing. Pass a command after --.")?;
            let status = keek::connect_exec::run(command, rest).map_err(anyhow::Error::msg)?;
            Ok(status)
        }
        Command::Mcp => {
            // Stdout carries MCP, so logs go to stderr.
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .init();
            tokio::runtime::Runtime::new()?
                .block_on(keek::mcp::serve())
                .map(|()| std::process::ExitCode::SUCCESS)
        }
        Command::Daemon => tokio::runtime::Runtime::new()?
            .block_on(run_daemon())
            .map(|()| std::process::ExitCode::SUCCESS),
    }
}

async fn run_daemon() -> Result<()> {
    let paths = Paths::from_env()?;
    fs::create_dir_all(paths.state_dir()).context("could not create the state folder")?;
    let log = File::options()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(paths.daemon_log())
        .context("could not open the daemon log")?;
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(log)
        .init();

    tracing::info!(
        version = keek::VERSION,
        pid = std::process::id(),
        "keek daemon starting"
    );
    match serve_daemon(&paths).await {
        Ok(exit) => {
            tracing::info!(?exit, "keek daemon exiting");
            Ok(())
        }
        Err(error) => {
            // The daemon's stderr is /dev/null, so the log is the only record of why.
            tracing::error!("keek daemon failed: {error:#}");
            Err(error)
        }
    }
}

async fn serve_daemon(paths: &Paths) -> Result<Exit> {
    let idle_limit_value = std::env::var(IDLE_LIMIT_ENV).ok();
    let idle_limit = daemon::idle_limit(idle_limit_value.as_deref())?;
    Ok(daemon::run(paths, idle_limit, MonotonicClock).await?)
}
