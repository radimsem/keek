use super::*;
use std::fs;

fn paths() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let session = temp.path().join("session");
    (temp, host, session)
}

#[test]
fn the_sessions_toggles_hold_bar_off() {
    let (_temp, host, session) = paths();
    fs::create_dir(&host).unwrap();
    prepare_session_toggles(&host, &session).unwrap();
    assert!(session.join(BAR_OFF).is_file());
}

#[test]
fn the_sessions_toggles_keep_the_developers_other_flags_including_nested_files() {
    let (_temp, host, session) = paths();
    fs::create_dir_all(host.join("hypr")).unwrap();
    fs::write(host.join("hypr/flags.lua"), "flags").unwrap();
    fs::write(host.join("suspend-off"), "on").unwrap();
    prepare_session_toggles(&host, &session).unwrap();
    assert_eq!(fs::read(session.join("hypr/flags.lua")).unwrap(), b"flags");
    assert_eq!(fs::read(session.join("suspend-off")).unwrap(), b"on");
}

#[test]
fn a_missing_developer_toggles_folder_still_gives_the_session_bar_off() {
    let (_temp, host, session) = paths();
    prepare_session_toggles(&host, &session).unwrap();
    assert!(session.join(BAR_OFF).is_file());
}

#[test]
fn the_developers_toggles_folder_is_left_as_it_was() {
    let (_temp, host, session) = paths();
    fs::create_dir(&host).unwrap();
    fs::write(host.join("other"), "content").unwrap();
    let listing = |p: &Path| {
        let mut entries: Vec<_> = fs::read_dir(p)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        entries.sort();
        entries
    };
    let before = listing(&host);
    prepare_session_toggles(&host, &session).unwrap();
    assert!(session.join(BAR_OFF).is_file());
    assert_eq!(before, listing(&host));
    assert!(!host.join(BAR_OFF).exists());
    assert_eq!(fs::read(host.join("other")).unwrap(), b"content");
}

#[test]
fn a_symlink_in_the_developers_toggles_is_not_followed() {
    let (_temp, host, session) = paths();
    fs::create_dir(&host).unwrap();
    std::os::unix::fs::symlink(&host, host.join("link")).unwrap();
    prepare_session_toggles(&host, &session).unwrap();
    assert!(session.join(BAR_OFF).is_file());
    assert!(!session.join("link").exists());
}

#[test]
fn the_developers_own_bar_off_is_kept_as_a_single_empty_flag() {
    let (_temp, host, session) = paths();
    fs::create_dir(&host).unwrap();
    fs::write(host.join(BAR_OFF), "").unwrap();
    prepare_session_toggles(&host, &session).unwrap();
    assert_eq!(fs::read(session.join(BAR_OFF)).unwrap(), b"");
    assert_eq!(fs::read_dir(&session).unwrap().count(), 1);
}

#[test]
fn an_unreadable_developer_toggle_is_skipped() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, host, session) = paths();
    fs::create_dir(&host).unwrap();
    let unreadable = host.join("unreadable");
    fs::write(&unreadable, "private").unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    let result = prepare_session_toggles(&host, &session);
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();
    result.unwrap();
    assert!(!session.join("unreadable").exists());
    assert!(session.join(BAR_OFF).is_file());
}

#[test]
fn a_developer_toggles_path_that_is_a_file_is_skipped() {
    let (_temp, host, session) = paths();
    fs::write(&host, "not a folder").unwrap();
    prepare_session_toggles(&host, &session).unwrap();
    assert!(session.join(BAR_OFF).is_file());
}

#[test]
fn a_read_only_developer_bar_off_does_not_prevent_the_session_flag() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, host, session) = paths();
    fs::create_dir(&host).unwrap();
    let flag = host.join(BAR_OFF);
    fs::write(&flag, "host").unwrap();
    fs::set_permissions(&flag, fs::Permissions::from_mode(0o444)).unwrap();
    prepare_session_toggles(&host, &session).unwrap();
    assert_eq!(fs::read(session.join(BAR_OFF)).unwrap(), b"");
}
