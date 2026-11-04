use super::*;

fn shell(script: &str) -> tokio::process::Child {
    tokio::process::Command::new("/usr/bin/sh")
        .args(["-c", script])
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[tokio::test]
async fn a_start_that_exits_is_reported_with_its_status() {
    let mut child = shell("exit 3");
    let start = tokio::time::timeout(
        Duration::from_secs(5),
        probe(&mut child, Duration::from_secs(5)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(start, StartProbe::Exited { status: 3 });
}

#[tokio::test]
async fn a_start_still_running_at_the_limit_is_reported_running_and_stopped() {
    let mut child = shell("sleep 30");
    let start = tokio::time::timeout(
        Duration::from_secs(5),
        probe(&mut child, Duration::from_millis(200)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(start, StartProbe::Running);
    assert!(
        child.try_wait().unwrap().is_some(),
        "the start command still runs"
    );
}

#[test]
fn a_log_tail_is_the_last_4_kib_with_test_credentials_masked() {
    let directory = tempfile::tempdir().unwrap();
    let credentials = Credentials::parse(
        &directory.path().join("demo.env"),
        "TEST_PASSWORD=made-up-secret\n",
    )
    .unwrap();
    let log = directory.path().join("start.log");
    let text = format!("{}\nlistening with made-up-secret\n", "x".repeat(5000));
    std::fs::write(&log, text).unwrap();
    let tail = masked_tail(&log, &credentials);
    assert!(tail.ends_with("listening with [masked]\n"), "{tail}");
    assert!(tail.len() <= 4096, "{}", tail.len());
    assert_eq!(
        masked_tail(&directory.path().join("absent.log"), &credentials),
        ""
    );
}

#[test]
fn a_test_credential_cut_by_the_tail_boundary_does_not_leak_in_part() {
    let directory = tempfile::tempdir().unwrap();
    let credentials = Credentials::parse(
        &directory.path().join("demo.env"),
        "TEST_PASSWORD=made-up-secret\n",
    )
    .unwrap();
    let log = directory.path().join("start.log");
    std::fs::write(
        &log,
        format!("{}made-up-secret{}", "x".repeat(100), "y".repeat(4089)),
    )
    .unwrap();
    let tail = masked_tail(&log, &credentials);
    assert!(!tail.contains("secret"), "{}", &tail[..40]);
}
