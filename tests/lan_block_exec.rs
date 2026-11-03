use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn refusal_reaches_sandbox_err(directory: &Path, expected: &str) -> std::io::Result<()> {
    let marker = directory.join("marker");
    let log = directory.join("sandbox.err");
    let status = Command::new(env!("CARGO_BIN_EXE_keek"))
        .args(["connect-exec", "--", "/bin/touch"])
        .arg(&marker)
        .env_clear()
        .env("PATH", directory)
        .stderr(std::fs::File::create(&log)?)
        .status()?;
    assert!(!status.success());
    assert!(!marker.exists());
    let error = std::fs::read_to_string(log)?;
    assert!(error.contains(expected), "{error}");
    Ok(())
}

#[test]
fn a_missing_ip_refuses_connect_exec_and_reports_to_sandbox_err() {
    let directory = tempfile::tempdir().unwrap();
    refusal_reaches_sandbox_err(directory.path(), "Can't start ip:").unwrap();
    let error = std::fs::read_to_string(directory.path().join("sandbox.err")).unwrap();
    assert!(error.contains("Install iproute2"), "{error}");
}

#[test]
fn a_failed_ip_rule_refuses_connect_exec_and_reports_to_sandbox_err() {
    let directory = tempfile::tempdir().unwrap();
    let ip = directory.path().join("ip");
    std::fs::write(
        &ip,
        "#!/bin/sh\necho 'Operation not permitted' >&2\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(ip, std::fs::Permissions::from_mode(0o755)).unwrap();
    refusal_reaches_sandbox_err(
        directory.path(),
        "ip -4 rule add to 169.254.1.1/32 lookup main pref 10",
    )
    .unwrap();
    let error = std::fs::read_to_string(directory.path().join("sandbox.err")).unwrap();
    assert!(error.contains("Operation not permitted"), "{error}");
}
