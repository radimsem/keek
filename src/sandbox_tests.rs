use super::*;

fn spec() -> SandboxSpec {
    SandboxSpec {
        credentials: Default::default(),
        project: "demo".into(),
        worktree: "/worktree".into(),
        tree: "/trees/demo".into(),
        layer: "/projects/key".into(),
        profile: crate::project::Profile {
            install: "npm ci".into(),
            start: "npm run dev".into(),
            ..Default::default()
        },
        home: "/home/test".into(),
        runtime_dir: "/run/user/42".into(),
        session_dir: "/cache/keek/sessions/demo_12-00-00".into(),
        executable: "/usr/bin/keek".into(),
        driver:
            "/home/test/.cua-driver/packages/releases/0.38.0-x86_64-unknown-linux-gnu/cua-driver"
                .into(),
        uid: 42,
        gid: 43,
        user: "test".into(),
        lang: "C.UTF-8".into(),
        nvidia_devices: vec!["/dev/nvidia0".into(), "/dev/nvidiactl".into()],
        toolchains: Default::default(),
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
    sandbox.prepare().unwrap();
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
    let session = std::fs::read_to_string(sandbox.session_dir.join("nested/session.sh")).unwrap();
    assert!(session.contains("hyprctl plugin list -j"));
    assert!(session.contains("hyprctl version -j"));
    assert!(session.contains("hl.exec_cmd('/keek/nested/start-app.sh')"));
    assert!(!session.contains("python3"));
    assert!(!session.contains("probe.sh"));
}

#[test]
fn preparing_a_session_always_writes_the_start_launcher() {
    let directory = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.home = directory.path().join("home");
    sandbox.session_dir = directory.path().join("session");
    sandbox.prepare().unwrap();
    assert!(sandbox.session_dir.join("nested/session.sh").is_file());
    assert!(sandbox.session_dir.join("nested/start-app.sh").exists());
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
    let expected: std::collections::BTreeMap<_, _> = ["PATH", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"]
        .into_iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (k.into(), Some(v))))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn project_session_layers_keep_changed_source_above_installed_files() {
    let args = spec().args();
    let expected: Vec<OsString> = [
        "--overlay-src",
        "/projects/key",
        "--overlay-src",
        "/trees/demo",
        "--tmp-overlay",
        "/keek/worktree",
    ]
    .map(Into::into)
    .into();
    assert!(args.windows(expected.len()).any(|a| a == expected));
}
#[test]
fn project_session_does_not_bind_the_real_worktree() {
    let args = spec().args();
    assert!(
        !args
            .windows(3)
            .any(|a| a == ["--bind", "/worktree", "/worktree"])
    );
}
#[test]
fn project_session_receives_the_project_name() {
    let args = spec().args();
    assert!(
        args.windows(3)
            .any(|a| a == ["--setenv", "KEEK_PROJECT", "demo"])
    );
}
#[test]
fn project_ports_are_forwarded_to_the_host() {
    let mut sandbox = spec();
    sandbox.profile.ports = vec![
        crate::project::HostPort(5432.try_into().unwrap()),
        crate::project::HostPort(6379.try_into().unwrap()),
    ];
    assert!(sandbox.args().windows(2).any(|a| a == ["-T", "5432,6379"]));
}

#[test]
fn project_install_uses_the_tree_and_persistent_upper() {
    let spec = spec();
    let args = spec.args_for(SandboxMode::Install {
        upper: Path::new("/build/upper"),
        work: Path::new("/build/work"),
        log: Path::new("/log"),
    });
    let expected: Vec<OsString> = [
        "--overlay-src",
        "/trees/demo",
        "--overlay",
        "/build/upper",
        "/build/work",
        "/keek/worktree",
        "--chdir",
        "/keek/worktree",
        "/usr/bin/bash",
        "-c",
        "npm ci",
    ]
    .map(Into::into)
    .into();
    assert!(args.ends_with(&expected));
}

#[test]
fn project_install_has_no_desktop_access() {
    let args = spec().args_for(SandboxMode::Install {
        upper: Path::new("/build/upper"),
        work: Path::new("/build/work"),
        log: Path::new("/log"),
    });
    assert!(args.windows(2).any(|a| a == ["-T", "none"]));
    assert!(!args.iter().any(|a| a == "connect-exec"
        || a == "--bind"
        || a == "--dev-bind"
        || a == "KEEK_SESSION"
        || a == "WAYLAND_DISPLAY"));
}

#[test]
fn install_and_session_mount_worktrees_at_the_same_fixed_path() {
    let spec = spec();
    let session = spec.args();
    let install = spec.args_for(SandboxMode::Install {
        upper: Path::new("/build/upper"),
        work: Path::new("/build/work"),
        log: Path::new("/log"),
    });
    assert!(
        session
            .windows(2)
            .any(|a| a == ["--tmp-overlay", "/keek/worktree"])
    );
    assert!(
        install
            .windows(4)
            .any(|a| a == ["--overlay", "/build/upper", "/build/work", "/keek/worktree"])
    );
    assert!(
        install
            .windows(2)
            .any(|a| a == ["--chdir", "/keek/worktree"])
    );
    assert!(
        session
            .windows(3)
            .any(|a| a == ["--setenv", "KEEK_WORKTREE", "/keek/worktree"])
    );
    assert!(
        install
            .windows(3)
            .any(|a| a == ["--setenv", "KEEK_WORKTREE", "/keek/worktree"])
    );
    assert!(!session.iter().chain(&install).any(|a| a == "/worktree"));
}
#[test]
fn credential_values_reach_the_sandbox_on_an_arguments_file_not_the_command_line() {
    let mut spec = spec();
    spec.credentials = crate::credentials::Credentials::parse(
        Path::new("fixture.env"),
        "PASSWORD=made-up-value\nHOME=made-up-home",
    )
    .unwrap();
    let args = spec.args();
    assert!(
        !args
            .iter()
            .any(|a| a == "made-up-value" || a == "made-up-home"),
        "a Test Credential value reached the command line"
    );
    let clear = args.iter().position(|a| a == "--clearenv").unwrap();
    let file = args.iter().position(|a| a == "--args").unwrap();
    assert_eq!(
        args[file + 1],
        OsString::from(crate::connect_exec::SANDBOX_ARGS_FD.to_string())
    );
    let home = args
        .windows(3)
        .position(|a| a == ["--setenv", "HOME", "/home/test"])
        .unwrap();
    assert!(clear < file && file < home);
}
#[test]
fn the_sandbox_command_carries_the_credential_values_for_connect_exec() {
    let directory = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.session_dir = directory.path().join("session");
    std::fs::create_dir_all(&sandbox.session_dir).unwrap();
    sandbox.credentials = crate::credentials::Credentials::parse(
        Path::new("fixture.env"),
        "PASSWORD=made-up-value\nOTHER=long enough",
    )
    .unwrap();
    let command = format!("{:?}", sandbox.command().unwrap());
    assert!(
        command.contains(crate::credentials::SANDBOX_ENV),
        "{command}"
    );
    assert!(
        command.contains("OTHER=long enough") && command.contains("PASSWORD=made-up-value"),
        "the sandbox command carries no Test Credential values"
    );
}
#[test]
fn install_carries_no_credential_value_in_arguments_or_environment() {
    let directory = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.session_dir = directory.path().join("session");
    std::fs::create_dir_all(&sandbox.session_dir).unwrap();
    sandbox.credentials =
        crate::credentials::Credentials::parse(Path::new("fixture.env"), "PASSWORD=made-up-value")
            .unwrap();
    let install = sandbox
        .install_command(
            Path::new("/upper"),
            Path::new("/work"),
            &directory.path().join("install.log"),
        )
        .unwrap();
    let shown = format!("{install:?}");
    assert!(!shown.contains("made-up-value"), "install carries a value");
    assert!(
        !shown.contains("--args"),
        "install asks for an arguments file"
    );
    assert!(
        !shown.contains(crate::credentials::SANDBOX_ENV),
        "install carries Test Credentials"
    );
}
#[test]
fn the_session_carries_the_start_command_for_the_launcher() {
    assert!(
        spec()
            .args()
            .windows(3)
            .any(|a| a == ["--setenv", "KEEK_START", "npm run dev"])
    );
    let launcher = include_str!("../assets/nested/start-app.sh");
    assert!(launcher.contains("bash -c \"$KEEK_START\""), "{launcher}");
    assert!(!launcher.contains("setup.sh"), "{launcher}");
}
#[test]
fn profile_env_reaches_install_and_session_before_credentials_and_keek_variables() {
    let mut spec = spec();
    spec.profile.env.insert("APP_ENV".into(), "test".into());
    let install = spec.args_for(SandboxMode::Install {
        upper: Path::new("/build/upper"),
        work: Path::new("/build/work"),
        log: Path::new("/log"),
    });
    for args in [spec.args(), install] {
        let env = args
            .windows(3)
            .position(|a| a == ["--setenv", "APP_ENV", "test"])
            .unwrap_or_else(|| panic!("no APP_ENV in {args:?}"));
        let home = args
            .windows(2)
            .position(|a| a == ["--setenv", "HOME"])
            .unwrap();
        assert!(env < home);
        if let Some(file) = args.iter().position(|a| a == "--args") {
            assert!(env < file);
        }
    }
}

#[test]
fn pinned_toolchains_are_mounted_read_only_and_lead_path_in_install_and_session() {
    use crate::toolchain::{Fetch, ResolvedTool, Tool, Toolchains};
    let mut spec = spec();
    let store = PathBuf::from("/home/test/.cache/keek/toolchains");
    spec.toolchains = Toolchains {
        store: store.clone(),
        tools: vec![ResolvedTool {
            tool: Tool::Rust,
            pin: "1.90".into(),
            version: "1.90.0".into(),
            bin_dirs: vec![store.join("cargo/bin")],
            fetch: Fetch::Present,
        }],
    };
    let install = spec.args_for(SandboxMode::Install {
        upper: Path::new("/build/upper"),
        work: Path::new("/build/work"),
        log: Path::new("/log"),
    });
    for (args, base) in [
        (
            spec.args(),
            "/keek/nested/bin:/usr/share/omarchy/bin:/usr/bin",
        ),
        (install, "/usr/bin"),
    ] {
        let path = format!("{}:{base}", store.join("cargo/bin").display());
        assert!(
            args.windows(3)
                .any(|a| a == ["--setenv", "PATH", path.as_str()]),
            "{args:?}"
        );
        let home = args
            .windows(2)
            .position(|a| a == ["--tmpfs", "/home/test"])
            .unwrap();
        let mount = args
            .windows(3)
            .position(|a| {
                a == [
                    OsString::from("--ro-bind"),
                    store.clone().into(),
                    store.clone().into(),
                ]
            })
            .unwrap_or_else(|| panic!("no read-only store in {args:?}"));
        assert!(home < mount);
        assert!(
            !args
                .windows(2)
                .any(|a| a[0] == "--bind" && a[1] == store.as_os_str())
        );
        assert!(
            args.windows(3)
                .any(|a| a == ["--setenv", "RUSTUP_TOOLCHAIN", "1.90.0"])
        );
    }
}

#[test]
fn the_start_probe_runs_start_with_the_lan_block_and_credentials_but_no_desktop() {
    let args = spec().args_for(SandboxMode::Probe {
        log: Path::new("/log"),
    });
    let connect = args.iter().position(|a| a == "connect-exec").unwrap();
    assert_eq!(args[connect + 1], "--no-display");
    assert!(args.windows(2).any(|a| a == ["--args", "4"]));
    assert!(!args.iter().any(|a| a == "--dev-bind"
        || a == "/dev/dri"
        || a == "/sys"
        || a == "/run/user/42"
        || a == "/keek/nested"
        || a == "KEEK_SESSION"
        || a == "KEEK_START"
        || a == "WAYLAND_DISPLAY"));
    let expected: Vec<OsString> = [
        "--overlay-src",
        "/projects/key",
        "--overlay-src",
        "/trees/demo",
        "--tmp-overlay",
        "/keek/worktree",
        "--chdir",
        "/keek/worktree",
        "/usr/bin/bash",
        "-c",
        "npm run dev",
    ]
    .map(Into::into)
    .into();
    assert!(args.ends_with(&expected), "{args:?}");
    assert!(
        args.windows(3)
            .any(|a| a == ["--setenv", "PATH", "/usr/share/omarchy/bin:/usr/bin"])
    );
}
