use std::ffi::OsString;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

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
        })
    }

    pub fn prepare(&self, start_command: Option<&str>) -> Result<(), SandboxError> {
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
        if let Some(command) = start_command {
            write_executable(
                &nested.join("start-app.sh"),
                &format!("#!/bin/bash\n{command}\n"),
            )?;
        }
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
            "nameserver 169.254.1.1\n",
        )?;
        Ok(())
    }

    pub fn command(&self) -> Result<Command, SandboxError> {
        let mut command = Command::new("pasta");
        command.env_clear();
        for name in ["PATH", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .args(self.args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(self.session_dir.join("sandbox.err"))?)
            .kill_on_drop(true);
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
        let mut args: Vec<OsString> = [
            "-f",
            "-q",
            "--config-net",
            "--dns-forward",
            "169.254.1.1",
            "--no-map-gw",
            "-t",
            "none",
            "-u",
            "none",
            "-T",
            "none",
            "-U",
            "none",
            "--",
        ]
        .map(OsString::from)
        .into();
        args.push(self.executable.as_os_str().to_owned());
        args.extend(["connect-exec", "--", "bwrap", "--unshare-user", "--uid"].map(OsString::from));
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
        for (key, value) in [
            ("HOME", self.home.as_os_str()),
            ("USER", self.user.as_ref()),
            ("LANG", self.lang.as_ref()),
            (
                "PATH",
                "/keek/nested/bin:/usr/share/omarchy/bin:/usr/bin".as_ref(),
            ),
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
            args.extend([OsString::from("--setenv"), key.into(), value.to_owned()]);
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
        args.extend(
            [
                "/run/systemd/resolve/stub-resolv.conf",
                "--dev",
                "/dev",
                "--dev-bind",
                "/dev/dri",
                "/dev/dri",
            ]
            .map(OsString::from),
        );
        for device in &self.nvidia_devices {
            args.extend([
                OsString::from("--dev-bind"),
                device.as_os_str().to_owned(),
                device.as_os_str().to_owned(),
            ]);
        }
        args.extend(
            [
                "--ro-bind",
                "/sys",
                "/sys",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                "--tmpfs",
            ]
            .map(OsString::from),
        );
        args.push(self.home.as_os_str().to_owned());
        args.extend(["--perms", "0700", "--tmpfs"].map(OsString::from));
        args.push(self.runtime_dir.as_os_str().to_owned());
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
mod tests {
    use super::*;

    fn spec() -> SandboxSpec {
        SandboxSpec {
            home: "/home/test".into(), runtime_dir: "/run/user/42".into(),
            session_dir: "/cache/keek/sessions/demo_12-00-00".into(),
            executable: "/usr/bin/keek".into(), driver: "/home/test/.cua-driver/packages/releases/0.38.0-x86_64-unknown-linux-gnu/cua-driver".into(),
            uid: 42, gid: 43, user: "test".into(), lang: "C.UTF-8".into(),
            nvidia_devices: vec!["/dev/nvidia0".into(), "/dev/nvidiactl".into()],
        }
    }

    #[test]
    fn preparing_a_session_writes_executable_assets_and_preserves_shell_settings() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let mut sandbox = spec();
        sandbox.home = directory.path().join("home");
        sandbox.session_dir = directory.path().join("session");
        std::fs::create_dir_all(sandbox.home.join(".config/omarchy")).unwrap();
        std::fs::write(
            sandbox.home.join(".config/omarchy/shell.json"),
            r#"{"theme":"light","idle":{"screensaver":30,"lock":60,"other":7}}"#,
        )
        .unwrap();
        sandbox
            .prepare(Some("chromium 'data:text/html,fixture'"))
            .unwrap();
        for asset in [
            "init.sh",
            "session.sh",
            "hyprland.lua",
            "bin/systemd-cat",
            "start-app.sh",
        ] {
            let path = sandbox.session_dir.join("nested").join(asset);
            assert!(path.is_file(), "missing {}", path.display());
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        let shell: serde_json::Value =
            serde_json::from_slice(&std::fs::read(sandbox.session_dir.join("shell.json")).unwrap())
                .unwrap();
        assert_eq!(
            shell,
            serde_json::json!({"theme":"light","idle":{"screensaver":0,"lock":0,"other":7}})
        );
        assert_eq!(
            std::fs::read_to_string(sandbox.session_dir.join("resolv.conf")).unwrap(),
            "nameserver 169.254.1.1\n"
        );
        let session =
            std::fs::read_to_string(sandbox.session_dir.join("nested/session.sh")).unwrap();
        assert!(session.contains("hyprctl plugin list -j"));
        assert!(session.contains("hyprctl version -j"));
        assert!(session.contains("hl.exec_cmd('/keek/nested/start-app.sh')"));
        assert!(!session.contains("python3"));
        assert!(!session.contains("probe.sh"));
    }

    #[test]
    fn preparing_a_session_without_a_test_command_does_not_start_an_app() {
        let directory = tempfile::tempdir().unwrap();
        let mut sandbox = spec();
        sandbox.home = directory.path().join("home");
        sandbox.session_dir = directory.path().join("session");
        sandbox.prepare(None).unwrap();
        assert!(sandbox.session_dir.join("nested/session.sh").is_file());
        assert!(!sandbox.session_dir.join("nested/start-app.sh").exists());
    }

    #[test]
    fn the_sandbox_preserves_the_network_and_namespace_argument_order() {
        let args = spec().args();
        let prefix: Vec<OsString> = [
            "-f",
            "-q",
            "--config-net",
            "--dns-forward",
            "169.254.1.1",
            "--no-map-gw",
            "-t",
            "none",
            "-u",
            "none",
            "-T",
            "none",
            "-U",
            "none",
            "--",
            "/usr/bin/keek",
            "connect-exec",
            "--",
            "bwrap",
            "--unshare-user",
            "--uid",
            "42",
            "--gid",
            "43",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--die-with-parent",
            "--new-session",
            "--clearenv",
        ]
        .map(OsString::from)
        .into();
        assert!(args.starts_with(&prefix), "actual arguments: {args:?}");
    }

    #[test]
    fn the_sandbox_mounts_its_assets_and_nvidia_devices_without_a_page_server() {
        let args = spec().args();
        let has = |words: &[&str]| {
            args.windows(words.len()).any(|part| {
                part.iter()
                    .map(|word| word.to_string_lossy())
                    .eq(words.iter().copied())
            })
        };
        assert!(has(&["--dev-bind", "/dev/nvidia0", "/dev/nvidia0"]));
        assert!(has(&["--dev-bind", "/dev/nvidiactl", "/dev/nvidiactl"]));
        assert!(has(&["--setenv", "CUA_DRIVER_RS_ENABLE_WAYLAND", "1"]));
        assert!(has(&[
            "--setenv",
            "CUA_DRIVER_RS_TELEMETRY_ENABLED",
            "false"
        ]));
        assert!(has(&[
            "--setenv",
            "ATSPI_DBUS_IMPLEMENTATION",
            "dbus-daemon"
        ]));
        assert!(has(&[
            "--ro-bind",
            "/cache/keek/sessions/demo_12-00-00/nested",
            "/keek/nested"
        ]));
        assert_eq!(args.last(), Some(&OsString::from("/keek/nested/init.sh")));
        assert!(!args.iter().any(|arg| arg == "KEEK_PAGE"));
    }
    #[test]
    fn pasta_inherits_only_host_display_environment() {
        let temp = tempfile::tempdir().unwrap();
        let mut spec = spec();
        spec.session_dir = temp.path().to_owned();
        let command = spec.command().unwrap();
        let command = command.as_std();
        assert!(format!("{command:?}").contains("env -i"), "{command:?}");
        let actual: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(k, v)| (k.to_os_string(), v.map(std::ffi::OsStr::to_os_string)))
            .collect();
        let expected: std::collections::BTreeMap<_, _> =
            ["PATH", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"]
                .into_iter()
                .filter_map(|k| std::env::var_os(k).map(|v| (k.into(), Some(v))))
                .collect();
        assert_eq!(actual, expected);
    }
}
