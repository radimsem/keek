use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use keek::timeline::{SessionTimeline, TimelineEvent};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::process::Command;
use tokio::sync::Mutex;

static LIVE_DESKTOP: Mutex<()> = Mutex::const_new(());
type Client = RunningService<RoleClient, ()>;
const START_COMMAND: &str = "chromium --ozone-platform=wayland --kiosk --no-first-run --no-default-browser-check --password-store=basic --user-data-dir=$HOME/chromium 'data:text/html,<html><title>keek fixture</title><body onclick=\"document.body.style.background=document.body.style.background===%27red%27?%27white%27:%27red%27\" style=\"margin:0;height:100vh\">click fixture</body></html>'";

struct Fixture {
    directory: TempDir,
    client: Client,
    sessions: Vec<String>,
    daemons: Vec<u32>,
    extra_clients: Vec<Client>,
}

impl Fixture {
    async fn start() -> Result<Self> {
        Self::start_with(&[]).await
    }

    async fn start_with(extra_env: &[(&str, &str)]) -> Result<Self> {
        let directory =
            tokio::task::spawn_blocking(|| tempfile::Builder::new().prefix("keek-live-").tempdir())
                .await??;
        let root = directory.path().to_owned();
        let runtime = root.join("run");
        let host_runtime = host_runtime()?;
        let runtime_copy = runtime.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir(&runtime_copy)?;
            std::os::unix::fs::symlink(host_runtime.join("hypr"), runtime_copy.join("hypr"))?;
            Ok(())
        })
        .await??;
        let config_root = root.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir_all(config_root.join("config"))?;
            std::fs::create_dir_all(config_root.join("videos"))?;
            std::fs::write(
                config_root.join("config/user-dirs.dirs"),
                format!(
                    "XDG_VIDEOS_DIR=\"{}\"\n",
                    config_root.join("videos").display()
                ),
            )?;
            Ok(())
        })
        .await??;
        let command = mcp_command(&root, extra_env)?;
        let client = ().serve(TokioChildProcess::new(command)?).await?;
        let pid = common::keek_status(&client).await?.pid;
        Ok(Self {
            directory,
            client,
            sessions: Vec::new(),
            daemons: vec![pid],
            extra_clients: Vec::new(),
        })
    }

    #[allow(dead_code, reason = "only the incomplete-session tests reconnect")]
    async fn reconnect(&mut self) -> Result<u32> {
        let command = mcp_command(self.directory.path(), &[])?;
        let client = ().serve(TokioChildProcess::new(command)?).await?;
        let status = common::keek_status(&client).await;
        self.extra_clients.push(client);
        let pid = status?.pid;
        if !self.daemons.contains(&pid) {
            self.daemons.push(pid);
        }
        Ok(pid)
    }

    async fn stop(self) -> Result<()> {
        for client in std::iter::once(self.client).chain(self.extra_clients) {
            // A killed keek mcp has nothing left to cancel.
            let _ = client.cancel().await;
        }
        let mut errors = Vec::new();
        for pid in self.daemons {
            if let Err(error) = common::stop_daemon(
                &self.directory.path().join("run"),
                pid,
                Duration::from_secs(5),
            )
            .await
            {
                errors.push(format!("daemon {pid}: {error:#}"));
            }
        }
        anyhow::ensure!(
            errors.is_empty(),
            "{}",
            errors.join("; ")
        );
        Ok(())
    }

    async fn worktree(&self, name: &str) -> Result<PathBuf> {
        self.worktree_with_start(name, START_COMMAND).await
    }

    async fn worktree_with_start(&self, name: &str, start: &str) -> Result<PathBuf> {
        let path = self.directory.path().join(name);
        let create = path.clone();
        tokio::task::spawn_blocking(move || std::fs::create_dir(create)).await??;
        run("git", &["init", "--quiet"], Some(&path)).await?;
        write_profile(self.directory.path(), name, &format!("start = {}\n", toml::Value::String(start.to_owned()))).await?;
        run(
            "git",
            &[
                "-c",
                "user.name=keek test",
                "-c",
                "user.email=keek-test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
            ],
            Some(&path),
        )
        .await?;
        Ok(path)
    }

    async fn open(&mut self, project: &str, worktree: &Path) -> Result<String> {
        // A fixture worktree carries the profile written under its own name.
        let profiles = self.directory.path().join("config/keek/projects");
        let source = profiles.join(format!("{}.toml", worktree.file_name().context("worktree has no name")?.to_string_lossy()));
        let target = profiles.join(format!("{project}.toml"));
        tokio::task::spawn_blocking(move || -> Result<()> {
            if !target.exists() && source.exists() {
                std::fs::copy(source, target)?;
            }
            Ok(())
        })
        .await??;
        let result = call(&self.client, "open_session", json!({"project":project,"worktree":worktree,"goal":"verify fixture clicks","checks":["fixture receives clicks"]})).await?;
        let name = result
            .get("session")
            .and_then(Value::as_str)
            .context("open_session returned no session name")?
            .to_owned();
        println!(
            "open_session returned ready Agent Session {name} for {}",
            worktree.display()
        );
        self.sessions.push(name.clone());
        Ok(name)
    }

    async fn click(&self, session: &str, count: usize) -> Result<u64> {
        let mut window = None;
        for _ in 0..150 {
            let result = call(&self.client, "list_windows", json!({"session":session})).await?;
            window = result
                .get("windows")
                .and_then(Value::as_array)
                .and_then(|windows| {
                    windows.iter().find_map(|window| {
                        window
                            .get("title")?
                            .as_str()?
                            .contains("keek fixture")
                            .then_some(())?;
                        Some((
                            window.get("pid")?.as_u64()?,
                            window.get("window_id")?.as_u64()?,
                        ))
                    })
                });
            if window.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let (pid, window_id) = window.context("list_windows never found Chromium")?;
        println!(
            "{session}: list_windows found Chromium pid={pid} window_id={window_id}; allowing 5 seconds for paint"
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
        call(
            &self.client,
            "get_window_state",
            json!({"session":session,"pid":pid,"window_id":window_id}),
        )
        .await?;
        for index in 0..count {
            call(
                &self.client,
                "click",
                json!({"session":session,"pid":pid,"window_id":window_id,"x":195,"y":400}),
            )
            .await?;
            println!("{session}: click {} completed at (195,400)", index + 1);
        }
        Ok(pid)
    }

    async fn close(&mut self, session: &str) -> Result<Records> {
        call(
            &self.client,
            "mark_check",
            json!({"session":session,"check":1,"outcome":"passed"}),
        )
        .await?;
        self.close_marked(session).await
    }
    async fn close_marked(&mut self, session: &str) -> Result<Records> {
        let result = call(
            &self.client,
            "close_session",
            json!({"session":session,"summary":"fixture clicks completed"}),
        )
        .await?;
        self.sessions.retain(|name| name != session);
        anyhow::ensure!(
            result.get("incomplete").is_none(),
            "close_session filed an Incomplete Session: {}",
            result["incomplete"]
        );
        let path = |key: &str| -> Result<PathBuf> {
            Ok(PathBuf::from(
                result["evidence"][key]
                    .as_str()
                    .context("missing Evidence path")?,
            ))
        };
        let records = Records {
            cache: self
                .directory
                .path()
                .join("cache/keek/sessions")
                .join(session),
            raw: path("raw")?,
            verification: path("verification")?,
            timeline: path("timeline")?,
            actions: path("actions")?,
            report: path("report")?,
            response: result,
        };
        println!(
            "{session}: closed; Evidence in {}",
            records.report.display()
        );
        Ok(records)
    }
    async fn finish(mut self) -> Result<()> {
        let mut errors = Vec::new();
        for session in std::mem::take(&mut self.sessions) {
            if let Err(error) = self.close(&session).await {
                errors.push(format!("{session}: {error:#}"));
            }
        }
        if let Err(error) = self.stop().await {
            errors.push(format!("daemon cleanup: {error:#}"));
        }
        anyhow::ensure!(
            errors.is_empty(),
            "Agent Session cleanup failed: {}",
            errors.join("; ")
        );
        Ok(())
    }
}

fn host_runtime() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is missing")?,
    ))
}

fn host_display() -> Result<PathBuf> {
    let display = PathBuf::from(
        std::env::var_os("WAYLAND_DISPLAY").context("WAYLAND_DISPLAY is missing")?,
    );
    Ok(if display.is_absolute() {
        display
    } else {
        host_runtime()?.join(display)
    })
}

/// The daemon inherits this environment from the first `keek mcp` that starts it.
fn mcp_command(root: &Path, extra_env: &[(&str, &str)]) -> Result<Command> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keek"));
    command
        .arg("mcp")
        .env("XDG_RUNTIME_DIR", root.join("run"))
        .env("WAYLAND_DISPLAY", host_display()?)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_VIDEOS_DIR", root.join("videos"))
        .env("KEEK_DAEMON_IDLE_MS", "100");
    for (name, value) in extra_env {
        command.env(name, value);
    }
    command.stderr(Stdio::inherit()).kill_on_drop(true);
    Ok(command)
}

async fn call(client: &Client, name: &str, arguments: Value) -> Result<Value> {
    let result = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned()).with_arguments(
                arguments
                    .as_object()
                    .context("arguments must be an object")?
                    .clone(),
            ),
        )
        .await?;
    let value = serde_json::to_value(&result)?;
    if value.get("isError").and_then(Value::as_bool) == Some(true) {
        bail!("{name} refused: {value}");
    }
    if let Some(value) = value.get("structuredContent") {
        return Ok(value.clone());
    }
    if let Some(text) = value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        && let Ok(parsed) = serde_json::from_str(text)
    {
        return Ok(parsed);
    }
    Ok(value)
}

async fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args).kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let result = command.output().await?;
    anyhow::ensure!(
        result.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}

async fn host_state() -> Result<Value> {
    let window: Value =
        serde_json::from_str(&run("hyprctl", &["activewindow", "-j"], None).await?)?;
    let workspace: Value =
        serde_json::from_str(&run("hyprctl", &["activeworkspace", "-j"], None).await?)?;
    Ok(
        json!({"address":window.get("address"),"workspace_id":workspace.get("id"),"workspace_name":workspace.get("name")}),
    )
}

async fn host_config() -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let root =
        PathBuf::from(std::env::var_os("HOME").context("HOME is missing")?).join(".config/hypr");
    tokio::task::spawn_blocking(move || {
        fn read(path: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) -> std::io::Result<()> {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    read(&path, files)?;
                } else if path.is_file() {
                    files.push((path.clone(), std::fs::read(path)?));
                }
            }
            Ok(())
        }
        let mut files = Vec::new();
        read(&root, &mut files)?;
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(files)
    })
    .await?
}

async fn assert_records(records: &Records, actions: usize, pid: u64) -> Result<()> {
    let recording = &records.raw;
    let duration = run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            recording.to_str().context("recording path is not UTF-8")?,
        ],
        None,
    )
    .await?
    .parse::<f64>()?;
    println!("{}: ffprobe duration={duration}s", recording.display());
    anyhow::ensure!(duration > 0.0, "Raw Recording duration is zero");
    let timeline_path = records.timeline.clone();
    let folder = records.actions.clone();
    let (timeline_actions, turns) = tokio::task::spawn_blocking(move || -> Result<_> {
        let timeline = SessionTimeline::parse(&std::fs::read_to_string(timeline_path)?)?;
        let calls: Vec<_> = timeline
            .events()
            .iter()
            .filter_map(|event| match event {
                TimelineEvent::Call {
                    tool, action: true, ..
                } => Some(tool.as_str()),
                TimelineEvent::Call { action: false, .. }
                | TimelineEvent::CheckMark { .. }
                | TimelineEvent::Reminder { .. }
                | TimelineEvent::Takeover { .. }
                | TimelineEvent::HelpRequest { .. } => None,
            })
            .collect();
        assert!(
            calls.iter().all(|tool| *tool == "click"),
            "unexpected Actions: {calls:?}"
        );
        let mut turns = 0;
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("turn-") && entry.path().is_dir() {
                let action: Value =
                    serde_json::from_slice(&std::fs::read(entry.path().join("action.json"))?)?;
                anyhow::ensure!(
                    action.to_string().contains("click"),
                    "Action Log turn does not contain a click"
                );
                anyhow::ensure!(
                    action.pointer("/arguments/pid").and_then(Value::as_u64) == Some(pid),
                    "Action Log turn has the wrong pid: {action}"
                );
                turns += 1;
            }
        }
        Ok((calls.len(), turns))
    })
    .await??;
    println!(
        "Session Timeline has {timeline_actions} Actions; Action Log has {turns} turn folders; expected {actions}"
    );
    anyhow::ensure!(
        timeline_actions == actions,
        "expected {actions} Actions, found {timeline_actions}"
    );
    anyhow::ensure!(turns == actions, "expected {actions} turns, found {turns}");
    Ok(())
}

async fn assert_processes_gone(folder: &Path) -> Result<()> {
    let marker = format!("KEEK_SESSION={}", folder.display());
    let folder = folder.to_string_lossy().into_owned();
    // Sandboxed processes keep the developer's uid. Root's login helpers also name
    // Hyprland, and their environment can't be read.
    let uid = run("id", &["-u"], None).await?;
    for attempt in 0..=20 {
        let result = Command::new("pgrep")
            .args(["-u", &uid, "-f", "pasta|bwrap|Hyprland|cua-driver"])
            .output()
            .await?;
        anyhow::ensure!(
            result.status.success() || result.status.code() == Some(1),
            "pgrep failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let pids: Vec<_> = String::from_utf8(result.stdout)?
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let marker = marker.clone();
        let folder = folder.clone();
        let survivors = tokio::task::spawn_blocking(move || {
            pids.into_iter()
                .filter(|pid| {
                    let base = PathBuf::from(format!("/proc/{pid}"));
                    let environ = std::fs::read(base.join("environ"));
                    let cmdline = std::fs::read(base.join("cmdline"));
                    let matches = environ.as_ref().is_ok_and(|bytes| {
                        bytes.split(|b| *b == 0).any(|v| v == marker.as_bytes())
                    }) || cmdline.as_ref().is_ok_and(|bytes| {
                        bytes.windows(folder.len()).any(|v| v == folder.as_bytes())
                    });
                    if matches {
                        return true;
                    }
                    if environ.is_ok() && cmdline.is_ok() {
                        return false;
                    }
                    match std::fs::read_to_string(base.join("stat")) {
                        Ok(stat) => !stat
                            .rsplit_once(") ")
                            .is_some_and(|(_, rest)| rest.starts_with('Z')),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                        Err(_) => true,
                    }
                })
                .collect::<Vec<_>>()
        })
        .await?;
        if survivors.is_empty() {
            return Ok(());
        }
        if attempt == 20 {
            bail!("Agent Session processes survived or could not be inspected: {survivors:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

struct Records {
    cache: PathBuf,
    raw: PathBuf,
    verification: PathBuf,
    timeline: PathBuf,
    actions: PathBuf,
    report: PathBuf,
    response: Value,
}

pub async fn write_profile(root: &Path, project: &str, text: &str) -> Result<()> {
    let folder = root.join("config/keek/projects");
    let path = folder.join(format!("{project}.toml"));
    let text = text.to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        std::fs::create_dir_all(folder)?;
        std::fs::write(path, text)?;
        Ok(())
    })
    .await??;
    Ok(())
}
