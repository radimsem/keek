use super::*;
use std::io::Write;
use std::time::Duration;

const SECRET: &str = "hunter2-made-up";

fn credentials() -> Credentials {
    Credentials::parse(
        Path::new("app.env"),
        &format!("KEEK_TEST_PASSWORD={SECRET}\n"),
    )
    .unwrap()
}

fn append(log: &Path, text: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

fn messages(receiver: &mut mpsc::Receiver<Event>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(Event::Progress { message }) = receiver.try_recv() {
        out.push(message);
    }
    out
}

async fn tail_for(
    progress: &OpenProgress,
    log: &Path,
    steps: &[(u64, &str)],
    total: u64,
) -> &'static str {
    let work = async {
        let mut at = 0;
        for (second, text) in steps {
            tokio::time::sleep(Duration::from_secs(second - at)).await;
            at = *second;
            append(log, text);
        }
        tokio::time::sleep(Duration::from_secs(total - at)).await;
        "done"
    };
    tokio::time::timeout(Duration::from_secs(600), progress.tailing(log, work))
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn the_latest_install_line_is_sent_at_most_once_every_ten_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("install.log");
    let (sender, mut receiver) = mpsc::channel(16);
    let progress = OpenProgress::new(Some(sender), credentials());
    let result = tail_for(
        &progress,
        &log,
        &[(1, "one\n"), (11, "two\n"), (12, "three\n")],
        35,
    )
    .await;
    assert_eq!(result, "done");
    assert_eq!(
        messages(&mut receiver),
        ["install: one", "install: three"],
        "one line at 10 s, the newest at 20 s, and nothing once the log stops changing"
    );
}

#[tokio::test(start_paused = true)]
async fn an_install_line_never_carries_a_test_credential_value() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("install.log");
    let (sender, mut receiver) = mpsc::channel(16);
    let progress = OpenProgress::new(Some(sender), credentials());
    tail_for(&progress, &log, &[(1, &format!("login {SECRET} ok\n"))], 15).await;
    assert_eq!(messages(&mut receiver), ["install: login [masked] ok"]);
}

#[tokio::test(start_paused = true)]
async fn a_long_install_line_is_capped() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("install.log");
    let (sender, mut receiver) = mpsc::channel(16);
    let progress = OpenProgress::new(Some(sender), credentials());
    tail_for(
        &progress,
        &log,
        &[(1, &format!("{}\n", "x".repeat(900)))],
        15,
    )
    .await;
    let sent = messages(&mut receiver);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].chars().count(), "install: ".len() + 200);
}

#[tokio::test(start_paused = true)]
async fn a_progress_bar_redraw_counts_as_the_latest_line() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("install.log");
    let (sender, mut receiver) = mpsc::channel(16);
    let progress = OpenProgress::new(Some(sender), credentials());
    tail_for(&progress, &log, &[(1, "10%\r50%\r90%\r")], 15).await;
    assert_eq!(messages(&mut receiver), ["install: 90%"]);
}

#[tokio::test(start_paused = true)]
async fn a_half_written_line_waits_for_its_end() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("install.log");
    let (sender, mut receiver) = mpsc::channel(16);
    let progress = OpenProgress::new(Some(sender), credentials());
    tail_for(
        &progress,
        &log,
        &[(1, "done\npassword=hunter"), (15, "2-made-up\n")],
        25,
    )
    .await;
    assert_eq!(
        messages(&mut receiver),
        ["install: done", "install: password=[masked]"]
    );
}

#[tokio::test(start_paused = true)]
async fn a_line_longer_than_the_tail_window_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("install.log");
    let (sender, mut receiver) = mpsc::channel(16);
    let progress = OpenProgress::new(Some(sender), credentials());
    tail_for(
        &progress,
        &log,
        &[(1, &format!("short\n{}\n", "y".repeat(9000)))],
        15,
    )
    .await;
    assert!(messages(&mut receiver).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_missing_log_sends_nothing_and_the_work_still_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let (sender, mut receiver) = mpsc::channel(16);
    let progress = OpenProgress::new(Some(sender), credentials());
    let result = tail_for(&progress, &dir.path().join("install.log"), &[], 25).await;
    assert_eq!(result, "done");
    assert!(messages(&mut receiver).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_phase_message_is_masked_and_a_full_queue_does_not_block() {
    let (sender, mut receiver) = mpsc::channel(1);
    let progress = OpenProgress::new(Some(sender), credentials());
    progress.send(&format!("Starting {SECRET}"));
    progress.send("dropped, the queue is full");
    assert_eq!(messages(&mut receiver), ["Starting [masked]"]);
}

#[tokio::test(start_paused = true)]
async fn without_a_sink_nothing_is_sent_and_the_work_runs() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("install.log");
    let progress = OpenProgress::default();
    progress.send("ignored");
    assert_eq!(tail_for(&progress, &log, &[(1, "one\n")], 25).await, "done");
}

#[tokio::test(start_paused = true)]
async fn a_credential_value_with_trailing_whitespace_is_masked_before_the_line_is_trimmed() {
    let padded = Credentials::parse(
        Path::new("app.env"),
        &format!("KEEK_TEST_PASSWORD=\"{SECRET} \"\n"),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("install.log");
    let (sender, mut receiver) = mpsc::channel(16);
    let progress = OpenProgress::new(Some(sender), padded);
    tail_for(
        &progress,
        &log,
        &[(1, &format!("password={SECRET} \n"))],
        15,
    )
    .await;
    let sent = messages(&mut receiver);
    assert_eq!(sent, ["install: password=[masked]"], "{sent:?}");
}
