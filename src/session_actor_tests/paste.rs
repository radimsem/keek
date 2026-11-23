use super::*;
use std::io::Read;

const SECRET: &str = "made-up-recovery-key-7Q2X";

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn files_holding(folder: &Path, needle: &str, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            files_holding(&path, needle, found);
        } else if metadata.is_file() && contains(&std::fs::read(&path).unwrap(), needle) {
            found.push(path);
        }
    }
}

#[tokio::test]
async fn a_carried_secret_reaches_the_session_fifo_and_no_evidence_or_cua_call() {
    let root = tempfile::tempdir().unwrap();
    let actor = start_actor(root.path(), "paste-secret", false).await;
    crate::takeover_clipboard::create_fifo(actor.folder.root()).unwrap();
    // Opened for reading and writing, as the sandbox reader opens it.
    let mut fifo = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(
            actor
                .folder
                .root()
                .join(crate::takeover_clipboard::FIFO_NAME),
        )
        .unwrap();
    let (frames, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buffer = [0u8; 4096];
        while !all.ends_with(b"clear\n") {
            match fifo.read(&mut buffer) {
                Ok(read) => all.extend_from_slice(&buffer[..read]),
                Err(_) => break,
            }
        }
        let _ = frames.send(all);
    });
    *actor.host_clipboard.lock().unwrap() = Some(SECRET);
    actor
        .control
        .commands
        .send(control::Command::TakeControl)
        .await
        .unwrap();
    let mut events = actor.events.clone();
    while !matches!(
        events.borrow_and_update().control,
        crate::ipc::ControlState::Takeover { .. }
    ) {
        events.changed().await.unwrap();
    }
    let held = tokio::spawn({
        let sender = actor.sender.clone();
        let gate = actor.control.gate.clone();
        async move { super::takeover::forward(&sender, &gate, "click").await }
    });
    actor
        .control
        .commands
        .send(control::Command::Done)
        .await
        .unwrap();
    let reply = format!("{:?}", held.await.unwrap());
    assert!(
        !contains(reply.as_bytes(), SECRET),
        "the hand-back carried the text"
    );
    let frames = tokio::task::spawn_blocking(move || {
        received.recv_timeout(std::time::Duration::from_secs(5))
    })
    .await
    .unwrap()
    .expect("the session FIFO never got a copy and a clear");
    assert!(
        contains(&frames, SECRET),
        "the text never reached the session FIFO, so its absence below proves nothing"
    );
    let calls = actor.calls.clone();
    let Response::Closed(value) = finish(actor).await else {
        panic!("expected filed evidence");
    };
    let timeline = Path::new(value["evidence"]["timeline"].as_str().unwrap());
    let mut found = Vec::new();
    files_holding(timeline.parent().unwrap(), SECRET, &mut found);
    files_holding(root.path(), SECRET, &mut found);
    assert!(found.is_empty(), "Evidence files hold the text: {found:?}");
    let calls = format!("{:?}", calls.lock().unwrap());
    assert!(
        !contains(calls.as_bytes(), SECRET),
        "a Cua call or host command carried the text"
    );
}
