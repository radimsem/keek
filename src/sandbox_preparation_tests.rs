#[test]
fn preparing_a_session_writes_toggles_with_bar_off() {
    let temp = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.home = temp.path().join("home");
    sandbox.session_dir = temp.path().join("session");
    let host = sandbox.home.join(".local/state/omarchy/toggles");
    std::fs::create_dir_all(&host).unwrap();
    std::fs::write(host.join("other"), "keep").unwrap();
    sandbox.prepare().unwrap();
    assert!(sandbox.session_dir.join("toggles/bar-off").is_file());
    assert_eq!(std::fs::read(host.join("other")).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(host).unwrap().count(), 1);
}

#[test]
fn the_nested_config_zeroes_gaps_borders_and_rounding_after_the_developers_config_loads() {
    let temp = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.home = temp.path().join("home");
    sandbox.session_dir = temp.path().join("session");
    sandbox.prepare().unwrap();
    let config = std::fs::read_to_string(sandbox.session_dir.join("nested/hyprland.lua")).unwrap();
    let load = config.find("dofile(os.getenv(\"HOME\")").unwrap();
    let override_at = config.find("general = { gaps_in = 0, gaps_out = 0, border_size = 0 }, decoration = { rounding = 0 }").unwrap();
    assert!(load < override_at);
}

#[test]
fn preparing_a_session_makes_the_clipboard_fifo_and_its_reader() {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let mut sandbox = spec();
    sandbox.home = temp.path().join("home");
    sandbox.session_dir = temp.path().join("session");
    sandbox.prepare().unwrap();
    let fifo = std::fs::symlink_metadata(
        sandbox
            .session_dir
            .join(crate::takeover_clipboard::FIFO_NAME),
    )
    .unwrap();
    assert!(fifo.file_type().is_fifo());
    assert_eq!(fifo.permissions().mode() & 0o777, 0o600);
    let reader = std::fs::metadata(sandbox.session_dir.join("nested/clipboard.sh")).unwrap();
    assert_eq!(reader.permissions().mode() & 0o111, 0o111);
    let session = std::fs::read_to_string(sandbox.session_dir.join("nested/session.sh")).unwrap();
    assert!(session.contains("/keek/nested/clipboard.sh"));
}
