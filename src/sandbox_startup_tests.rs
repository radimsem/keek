use super::*;

#[test]
fn a_session_skips_the_startup_cover_with_its_own_marker() {
    let directory = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.home = directory.path().join("home");
    sandbox.session_dir = directory.path().join("session");
    sandbox.prepare().unwrap();
    let marker = sandbox.session_dir.join("background-intro.session-id");
    let value = std::fs::read_to_string(&marker).expect("session marker is missing");
    assert!(!value.trim().is_empty());
    let args = sandbox.args();
    assert!(args.windows(3).any(|args| {
        args == [
            OsString::from("--ro-bind"),
            marker.as_os_str().to_owned(),
            sandbox
                .home
                .join(".local/state/omarchy/background-intro.session-id")
                .into(),
        ]
    }));
    assert!(args.windows(3).any(|args| args
        == [
            OsString::from("--setenv"),
            "OMARCHY_SESSION_ID".into(),
            value.trim().into(),
        ]));
}

#[test]
fn a_probe_or_install_has_no_startup_cover_marker() {
    let sandbox = spec();
    for mode in [
        SandboxMode::Probe {
            log: Path::new("probe.log"),
        },
        SandboxMode::Install {
            upper: Path::new("upper"),
            work: Path::new("work"),
            log: Path::new("install.log"),
        },
    ] {
        let args = sandbox.args_for(mode);
        assert!(!args.iter().any(|arg| {
            arg == "OMARCHY_SESSION_ID"
                || arg
                    .to_string_lossy()
                    .contains("background-intro.session-id")
        }));
    }
}

#[test]
fn a_session_refuses_a_missing_omarchy_config_folder_before_starting() {
    let directory = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.home = directory.path().join("home");
    sandbox.session_dir = directory.path().join("session");
    sandbox.prepare().unwrap();
    for missing in [".config/hypr", ".config/omarchy"] {
        let path = sandbox.home.join(missing);
        let error = sandbox
            .command()
            .expect_err("missing config must be refused")
            .to_string();
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(error.contains("Hyprland and Omarchy config"), "{error}");
        std::fs::create_dir_all(path).unwrap();
    }
    sandbox.command().unwrap();
}

#[test]
fn a_probe_or_install_builds_without_omarchy_config_folders() {
    let directory = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.home = directory.path().join("home");
    sandbox.session_dir = directory.path().join("session");
    sandbox.prepare().unwrap();
    sandbox
        .probe_command(&directory.path().join("probe.log"))
        .unwrap();
    sandbox
        .install_command(
            directory.path(),
            directory.path(),
            &directory.path().join("install.log"),
        )
        .unwrap();
}
