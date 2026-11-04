use std::ffi::OsString;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

pub(crate) const DNS_FORWARDER: &str = "169.254.1.1";

// Console scripts in a shared Project Layer need the same absolute worktree path.
const SANDBOX_WORKTREE: &str = "/keek/worktree";

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("Can't prepare the Agent Session: {0}. Check that its session folder is writable.")]
    Io(#[from] io::Error),
    #[error(
        "Can't read Omarchy's shell settings: {0}. Correct shell.json before opening an Agent Session."
    )]
    Json(#[from] serde_json::Error),
    #[error(
        "Can't prepare the Agent Session because {0} is missing. Run keek from your desktop session."
    )]
    MissingEnvironment(&'static str),
}

#[derive(Debug, Clone)]
pub struct SandboxSpec {
    pub credentials: crate::credentials::Credentials,
    pub project: String,
    pub worktree: PathBuf,
    pub tree: PathBuf,
    pub layer: PathBuf,
    pub profile: crate::project::Profile,
    pub home: PathBuf,
    pub runtime_dir: PathBuf,
    pub session_dir: PathBuf,
    pub executable: PathBuf,
    pub driver: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub user: String,
    pub lang: String,
    pub nvidia_devices: Vec<PathBuf>,
    pub toolchains: crate::toolchain::Toolchains,
}

#[derive(Clone, Copy)]
enum SandboxMode<'a> {
    Session,
    Install {
        upper: &'a Path,
        work: &'a Path,
        log: &'a Path,
    },
    Probe {
        log: &'a Path,
    },
}

impl SandboxSpec {
    pub fn from_env(session_dir: &Path, driver: &Path) -> Result<Self, SandboxError> {
        let environment =
            |name| std::env::var_os(name).ok_or(SandboxError::MissingEnvironment(name));
        let home = PathBuf::from(environment("HOME")?);
        // SAFETY: getuid takes no pointers and has no preconditions.
        let uid = unsafe { libc::getuid() };
        // The plugin's socket path has to fit in sun_path, and a longer runtime
        // folder only fails at the first click. This one is a private tmpfs, so it
        // doesn't need to match the host's.
        let runtime_dir = PathBuf::from(format!("/run/user/{uid}"));
        let mut nvidia_devices = Vec::new();
        for entry in std::fs::read_dir("/dev")? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("nvidia") {
                nvidia_devices.push(entry.path());
            }
        }
        nvidia_devices.sort();
        Ok(Self {
            credentials: Default::default(),
            project: String::new(),
            worktree: PathBuf::new(),
            tree: PathBuf::new(),
            layer: PathBuf::new(),
            profile: Default::default(),
            home,
            runtime_dir,
            session_dir: session_dir.to_owned(),
            executable: std::env::current_exe()?,
            driver: driver.to_owned(),
            uid,
            // SAFETY: getgid takes no pointers and has no preconditions.
            gid: unsafe { libc::getgid() },
            user: environment("USER")?.to_string_lossy().into_owned(),
            lang: std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".to_owned()),
            nvidia_devices,
            toolchains: Default::default(),
        })
    }

    pub fn prepare(&self) -> Result<(), SandboxError> {
        let nested = self.session_dir.join("nested");
        std::fs::create_dir_all(nested.join("bin"))?;
        for (name, content) in [
            ("init.sh", include_str!("../assets/nested/init.sh")),
            ("session.sh", include_str!("../assets/nested/session.sh")),
            (
                "hyprland.lua",
                include_str!("../assets/nested/hyprland.lua"),
            ),
            (
                "bin/systemd-cat",
                include_str!("../assets/nested/bin/systemd-cat"),
            ),
        ] {
            write_executable(&nested.join(name), content)?;
        }
        write_executable(
            &nested.join("start-app.sh"),
            include_str!("../assets/nested/start-app.sh"),
        )?;
        let path = self.home.join(".config/omarchy/shell.json");
        let mut shell: serde_json::Value = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
            Err(error) => return Err(error.into()),
        };
        let object = shell.as_object_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "shell.json must contain an object",
            )
        })?;
        let idle = object
            .entry("idle")
            .or_insert_with(|| serde_json::json!({}));
        let idle = idle.as_object_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "shell.json idle must contain an object",
            )
        })?;
        idle.insert("screensaver".to_owned(), 0.into());
        idle.insert("lock".to_owned(), 0.into());
        std::fs::write(
            self.session_dir.join("shell.json"),
            serde_json::to_vec_pretty(&shell)?,
        )?;
        std::fs::write(
            self.session_dir.join("resolv.conf"),
            format!("nameserver {DNS_FORWARDER}\n"),
        )?;
        Ok(())
    }

    pub fn command(&self) -> Result<Command, SandboxError> {
        self.command_with(SandboxMode::Session)
    }

    pub fn install_command(
        &self,
        upper: &Path,
        work: &Path,
        log: &Path,
    ) -> Result<Command, SandboxError> {
        self.command_with(SandboxMode::Install { upper, work, log })
    }

    /// Runs `start` in the session's sandbox, network and LAN block, with no desktop, writing its output to `log`.
    pub fn probe_command(&self, log: &Path) -> Result<Command, SandboxError> {
        self.command_with(SandboxMode::Probe { log })
    }

    fn command_with(&self, mode: SandboxMode<'_>) -> Result<Command, SandboxError> {
        let mut command = Command::new("pasta");
        command.env_clear();
        for name in ["PATH", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        if matches!(mode, SandboxMode::Session | SandboxMode::Probe { .. })
            && let Some(values) = self.credentials.sandbox_env()
        {
            command.env(crate::credentials::SANDBOX_ENV, values);
        }
        command
            .args(self.args_for(mode))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(self.session_dir.join("sandbox.err"))?)
            .kill_on_drop(true);
        if let SandboxMode::Install { log, .. } | SandboxMode::Probe { log } = mode {
            let file = std::fs::File::create(log)?;
            command
                .stdin(Stdio::null())
                .stdout(file.try_clone()?)
                .stderr(file);
        }
        // SAFETY: prctl takes a signal number and reads no pointers.
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(command)
    }

    pub fn args(&self) -> Vec<OsString> {
        self.args_for(SandboxMode::Session)
    }

    fn args_for(&self, mode: SandboxMode<'_>) -> Vec<OsString> {
        let ports = self.profile.pasta_ports();
        let mut args: Vec<OsString> = [
            "-f",
            "-q",
            "--config-net",
            "--dns-forward",
            DNS_FORWARDER,
            "--no-map-gw",
            "-t",
            "none",
            "-u",
            "none",
            "-T",
            &ports,
            "-U",
            "none",
            "--",
        ]
        .map(OsString::from)
        .into();
        match mode {
            SandboxMode::Session => {
                args.push(self.executable.as_os_str().to_owned());
                args.extend(["connect-exec", "--"].map(OsString::from));
            }
            SandboxMode::Probe { .. } => {
                args.push(self.executable.as_os_str().to_owned());
                args.extend(["connect-exec", "--no-display", "--"].map(OsString::from));
            }
            SandboxMode::Install { .. } => {}
        }
        args.extend(["bwrap", "--unshare-user", "--uid"].map(OsString::from));
        args.push(self.uid.to_string().into());
        args.push("--gid".into());
        args.push(self.gid.to_string().into());
        args.extend(
            [
                "--unshare-pid",
                "--unshare-ipc",
                "--unshare-uts",
                "--die-with-parent",
                "--new-session",
                "--clearenv",
            ]
            .map(OsString::from),
        );
        // Before the Test Credentials and keek's own variables, so both win a clashing name.
        for (name, value) in &self.profile.env {
            args.extend([OsString::from("--setenv"), name.into(), value.into()]);
        }
        for (name, value) in self.toolchains.env() {
            args.extend([OsString::from("--setenv"), name.into(), value]);
        }
        if matches!(mode, SandboxMode::Session | SandboxMode::Probe { .. }) {
            // The file parses where this option stands, so the required variables below still win a clashing name.
            args.extend([
                OsString::from("--args"),
                crate::connect_exec::SANDBOX_ARGS_FD.to_string().into(),
            ]);
        }
        for (key, value) in [
            ("HOME", self.home.as_os_str()),
            ("USER", self.user.as_ref()),
            ("LANG", self.lang.as_ref()),
            ("OMARCHY_PATH", "/usr/share/omarchy".as_ref()),
            ("XDG_RUNTIME_DIR", self.runtime_dir.as_os_str()),
            ("XDG_SESSION_TYPE", "wayland".as_ref()),
            ("XDG_CURRENT_DESKTOP", "Hyprland".as_ref()),
            ("ATSPI_DBUS_IMPLEMENTATION", "dbus-daemon".as_ref()),
            ("KEEK_SESSION", self.session_dir.as_os_str()),
            ("KEEK_DRIVER", self.driver.as_os_str()),
            ("CUA_DRIVER_RS_ENABLE_WAYLAND", "1".as_ref()),
            ("CUA_DRIVER_RS_TELEMETRY_ENABLED", "false".as_ref()),
        ] {
            let shared = match mode {
                SandboxMode::Session => true,
                // Omarchy's scripts on the probe's PATH read OMARCHY_PATH.
                SandboxMode::Probe { .. } => {
                    matches!(key, "HOME" | "USER" | "LANG" | "OMARCHY_PATH")
                }
                SandboxMode::Install { .. } => matches!(key, "HOME" | "USER" | "LANG"),
            };
            if shared {
                args.extend([OsString::from("--setenv"), key.into(), value.to_owned()]);
            }
        }
        // The probe has no nested desktop, so it has no /keek/nested/bin.
        let path = self.toolchains.path(match mode {
            SandboxMode::Session => "/keek/nested/bin:/usr/share/omarchy/bin:/usr/bin",
            SandboxMode::Probe { .. } => "/usr/share/omarchy/bin:/usr/bin",
            SandboxMode::Install { .. } => "/usr/bin",
        });
        args.extend([OsString::from("--setenv"), "PATH".into(), path]);
        args.extend([
            OsString::from("--setenv"),
            "KEEK_PROJECT".into(),
            self.project.clone().into(),
        ]);
        args.extend([
            OsString::from("--setenv"),
            "KEEK_WORKTREE".into(),
            SANDBOX_WORKTREE.into(),
        ]);
        if matches!(mode, SandboxMode::Session) {
            args.extend([
                OsString::from("--setenv"),
                "KEEK_START".into(),
                self.profile.start.clone().into(),
            ]);
        }
        args.extend(
            [
                "--ro-bind",
                "/usr",
                "/usr",
                "--symlink",
                "usr/bin",
                "/bin",
                "--symlink",
                "usr/bin",
                "/sbin",
                "--symlink",
                "usr/lib",
                "/lib",
                "--symlink",
                "usr/lib",
                "/lib64",
                "--ro-bind",
                "/etc",
                "/etc",
                "--dir",
                "/run/systemd/resolve",
                "--ro-bind",
            ]
            .map(OsString::from),
        );
        args.push(self.session_dir.join("resolv.conf").into());
        args.extend(["/run/systemd/resolve/stub-resolv.conf", "--dev", "/dev"].map(OsString::from));
        if matches!(mode, SandboxMode::Session) {
            args.extend(["--dev-bind", "/dev/dri", "/dev/dri"].map(OsString::from));
            for device in &self.nvidia_devices {
                args.extend([
                    OsString::from("--dev-bind"),
                    device.as_os_str().to_owned(),
                    device.as_os_str().to_owned(),
                ]);
            }
        }
        if matches!(mode, SandboxMode::Session) {
            args.extend(["--ro-bind", "/sys", "/sys"].map(OsString::from));
        }
        args.extend(
            [
                "--dir", "/keek", "--proc", "/proc", "--tmpfs", "/tmp", "--tmpfs",
            ]
            .map(OsString::from),
        );
        args.push(self.home.as_os_str().to_owned());
        if !self.toolchains.tools.is_empty() {
            // Read-only, so an install or the App Under Test can't change a toolchain another Project uses.
            args.extend([
                OsString::from("--ro-bind"),
                self.toolchains.store.clone().into(),
                self.toolchains.store.clone().into(),
            ]);
        }
        if matches!(mode, SandboxMode::Session) {
            args.extend(["--perms", "0700", "--tmpfs"].map(OsString::from));
            args.push(self.runtime_dir.as_os_str().to_owned());
        }
        if let SandboxMode::Install { upper, work, .. } = mode {
            args.extend([
                OsString::from("--overlay-src"),
                self.tree.clone().into(),
                "--overlay".into(),
                upper.into(),
                work.into(),
                SANDBOX_WORKTREE.into(),
                "--chdir".into(),
                SANDBOX_WORKTREE.into(),
                "/usr/bin/bash".into(),
                "-c".into(),
                self.profile.install.clone().into(),
            ]);
            return args;
        }
        // The tree wins over install writes to tracked paths so the Verifier sees the agent's change.
        args.extend([
            OsString::from("--overlay-src"),
            self.layer.clone().into(),
            "--overlay-src".into(),
            self.tree.clone().into(),
            "--tmp-overlay".into(),
            SANDBOX_WORKTREE.into(),
            "--chdir".into(),
            SANDBOX_WORKTREE.into(),
        ]);
        if let SandboxMode::Probe { .. } = mode {
            args.extend([
                OsString::from("/usr/bin/bash"),
                "-c".into(),
                self.profile.start.clone().into(),
            ]);
            return args;
        }
        for path in [".config/hypr", ".config/omarchy"] {
            let path = self.home.join(path);
            args.extend([
                OsString::from("--ro-bind"),
                path.as_os_str().to_owned(),
                path.into(),
            ]);
        }
        args.extend([
            OsString::from("--ro-bind"),
            self.session_dir.join("shell.json").into(),
            self.home.join(".config/omarchy/shell.json").into(),
        ]);
        for path in ["current", "toggles", "settings"] {
            let path = self.home.join(".local/state/omarchy").join(path);
            args.extend([
                OsString::from("--ro-bind-try"),
                path.as_os_str().to_owned(),
                path.into(),
            ]);
        }
        args.extend([
            OsString::from("--bind"),
            self.session_dir.as_os_str().to_owned(),
            self.session_dir.as_os_str().to_owned(),
        ]);
        if let Some(directory) = self.driver.parent() {
            args.extend([
                OsString::from("--ro-bind"),
                directory.as_os_str().to_owned(),
                directory.as_os_str().to_owned(),
            ]);
        }
        args.extend([
            OsString::from("--ro-bind"),
            self.session_dir.join("nested").into(),
            OsString::from("/keek/nested"),
            OsString::from("/keek/nested/init.sh"),
        ]);
        args
    }
}

fn write_executable(path: &Path, content: &str) -> io::Result<()> {
    std::fs::write(path, content)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
