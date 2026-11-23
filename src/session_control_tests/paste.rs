use super::*;
use crate::ipc::TakeoverPhase;
use crate::takeover_clipboard::{CarriedText, ClipboardError, ClipboardFuture, NestedClipboard};

const SECRET: &str = "made-up-recovery-key-7Q2X";
const SECOND: &str = "made-up-otp-904417";
const TEXT_TYPES: &str = "text/plain;charset=utf-8\ntext/plain\nUTF8_STRING";

#[derive(Clone, PartialEq)]
enum Nested {
    Copy(Vec<u8>),
    Clear,
}
// Lengths only, so a failing assertion never prints the carried text.
impl std::fmt::Debug for Nested {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Nested::Copy(bytes) => write!(formatter, "Copy(<{} bytes>)", bytes.len()),
            Nested::Clear => write!(formatter, "Clear"),
        }
    }
}

#[derive(Default)]
struct Recorder {
    events: StdMutex<Vec<Nested>>,
    copy_fails: bool,
    clear_gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
impl NestedClipboard for Recorder {
    fn copy(&self, text: CarriedText) -> ClipboardFuture<'_> {
        Box::pin(async move {
            if self.copy_fails {
                return Err(ClipboardError::Nested(std::io::Error::from_raw_os_error(
                    libc::ENXIO,
                )));
            }
            self.events
                .lock()
                .unwrap()
                .push(Nested::Copy(text.as_bytes().to_vec()));
            Ok(())
        })
    }
    fn clear(&self) -> ClipboardFuture<'_> {
        Box::pin(async move {
            if let Some((entered, release)) = &self.clear_gate {
                entered.notify_one();
                release.notified().await;
            }
            self.events.lock().unwrap().push(Nested::Clear);
            Ok(())
        })
    }
}

struct Host {
    calls: Arc<StdMutex<Vec<String>>>,
    types: &'static str,
    text: Arc<StdMutex<&'static str>>,
    paste_fails: bool,
}
impl ProcessRunner for Host {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a std::path::Path>,
    ) -> crate::process::ProcessFuture<'a> {
        Box::pin(async move {
            self.output_bytes(program, args, cwd)
                .await
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
    }
    fn output_bytes<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> crate::process::BytesFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            if program == "wl-paste" {
                if self.paste_fails {
                    return Err(ProcessError::Failed {
                        program: program.into(),
                        message: "exit status: 1: Failed to connect to a Wayland server".into(),
                    });
                }
                if args.contains(&"--list-types") {
                    return Ok(self.types.as_bytes().to_vec());
                }
                return Ok(self.text.lock().unwrap().as_bytes().to_vec());
            }
            Ok(match args.first().copied() {
                Some("clients") => r#"[{"class":"editor","address":"0xdef","focusHistoryID":1},{"class":"aquamarine","address":"0xabc","focusHistoryID":0}]"#,
                Some("activewindow") => r#"{"address":"0xdef"}"#,
                Some("binds") => "[]",
                _ => "ok",
            }
            .as_bytes()
            .to_vec())
        })
    }
}

struct Paste {
    control: Control,
    task: tokio::task::JoinHandle<()>,
    calls: Arc<StdMutex<Vec<String>>>,
    text: Arc<StdMutex<&'static str>>,
    snapshots: watch::Receiver<SessionSnapshot>,
    clipboard: Arc<Recorder>,
}

fn start(types: &'static str, paste_fails: bool, clipboard: Recorder) -> Paste {
    let (control, receiver) = Control::new();
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let text = Arc::new(StdMutex::new(SECRET));
    let clipboard = Arc::new(clipboard);
    let (events, snapshots) = watch::channel(SessionSnapshot {
        control: ControlState::Verifier,
        state: crate::ipc::SessionState::Open,
        checks: vec![],
        reminded: false,
        last_action: None,
    });
    let controller = Controller {
        control: control.clone(),
        runner: Arc::new(Host {
            calls: calls.clone(),
            types,
            text: text.clone(),
            paste_fails,
        }),
        compositor: Arc::new(FakeCompositor(StdMutex::new(None))),
        clock: Arc::new(crate::clock::MonotonicClock),
        events,
        opened: Instant::now(),
        window: Some(WindowAddress("0xabc".into())),
        project: "app".into(),
        session: "app-12-00-00".parse().unwrap(),
        clipboard: clipboard.clone(),
    };
    Paste {
        control,
        task: tokio::spawn(controller.run(receiver, Box::pin(std::future::pending()))),
        calls,
        text,
        snapshots,
        clipboard,
    }
}

impl Paste {
    async fn send(&self, command: Command) {
        self.control.commands.send(command).await.unwrap();
    }
    async fn reach(&mut self, paused: Option<bool>) {
        loop {
            let reached = match &self.snapshots.borrow_and_update().control {
                ControlState::Takeover { phase, .. } => paused
                    .is_some_and(|paused| matches!(phase, TakeoverPhase::Paused { .. }) == paused),
                ControlState::HandBack { .. } => paused.is_none(),
                ControlState::Verifier | ControlState::HelpRequest { .. } => false,
            };
            if reached {
                return;
            }
            self.snapshots.changed().await.unwrap();
        }
    }
    fn nested(&self) -> Vec<Nested> {
        self.clipboard.events.lock().unwrap().clone()
    }
    fn held(&self) -> tokio::task::JoinHandle<crate::session::gate::Entry> {
        let gate = self.control.gate.clone();
        tokio::spawn(async move { gate.enter().await })
    }
    async fn stop(self) {
        self.control.stop().await;
        self.task.await.unwrap();
    }
}

async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

fn copy(text: &str) -> Nested {
    Nested::Copy(text.as_bytes().to_vec())
}

#[tokio::test]
async fn take_control_and_each_resume_carry_the_host_text_into_the_session() {
    let mut paste = start(TEXT_TYPES, false, Recorder::default());
    paste.send(Command::TakeControl).await;
    paste.reach(Some(false)).await;
    assert_eq!(
        paste.nested(),
        [copy(SECRET)],
        "Take control carried nothing"
    );
    paste.send(Command::StepAway).await;
    paste.reach(Some(true)).await;
    assert_eq!(paste.nested(), [copy(SECRET)], "Step away carried text");
    *paste.text.lock().unwrap() = SECOND;
    paste.send(Command::Resume).await;
    paste.reach(Some(false)).await;
    assert_eq!(
        paste.nested(),
        [copy(SECRET), copy(SECOND)],
        "Resume did not carry the host's current text"
    );
    paste.stop().await;
}

#[tokio::test]
async fn a_clipboard_without_text_is_skipped_and_the_takeover_runs() {
    let mut paste = start("image/png\nimage/jpeg", false, Recorder::default());
    paste.send(Command::TakeControl).await;
    paste.reach(Some(false)).await;
    assert!(
        paste.nested().is_empty(),
        "non-text content was carried: {:?}",
        paste.nested()
    );
    let reads: Vec<_> = paste
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with("wl-paste"))
        .cloned()
        .collect();
    assert_eq!(reads, ["wl-paste --list-types"]);
    let call = paste.held();
    paste.send(Command::Done).await;
    paste.reach(None).await;
    tokio::time::timeout(std::time::Duration::from_secs(1), call)
        .await
        .expect("Done did not release the held call")
        .unwrap();
    paste.stop().await;
}

#[tokio::test]
async fn done_clears_the_nested_clipboard_before_releasing_the_held_call() {
    for paused in [false, true] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let mut paste = start(
            TEXT_TYPES,
            false,
            Recorder {
                clear_gate: Some((entered.clone(), release.clone())),
                ..Recorder::default()
            },
        );
        paste.send(Command::TakeControl).await;
        paste.reach(Some(false)).await;
        if paused {
            paste.send(Command::StepAway).await;
            paste.reach(Some(true)).await;
        }
        let call = paste.held();
        paste.send(Command::Done).await;
        tokio::time::timeout(std::time::Duration::from_secs(1), entered.notified())
            .await
            .unwrap_or_else(|_| {
                panic!("Done never cleared the nested clipboard (paused: {paused})")
            });
        settle().await;
        assert!(
            !call.is_finished(),
            "the held call ran before the clear finished (paused: {paused})"
        );
        assert!(
            !matches!(
                paste.snapshots.borrow().control,
                ControlState::HandBack { .. }
            ),
            "the hand-back was published before the clear finished (paused: {paused})"
        );
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), call)
            .await
            .expect("the held call stayed held after the clear")
            .unwrap();
        assert_eq!(paste.nested(), [copy(SECRET), Nested::Clear]);
        paste.stop().await;
    }
}

#[tokio::test]
async fn the_carried_text_stays_out_of_command_lines_spans_and_snapshots() {
    let mut paste = start(TEXT_TYPES, false, Recorder::default());
    let mut seen = Vec::new();
    paste.send(Command::TakeControl).await;
    paste.reach(Some(false)).await;
    seen.push(format!("{:?}", *paste.snapshots.borrow()));
    paste.send(Command::StepAway).await;
    paste.reach(Some(true)).await;
    seen.push(format!("{:?}", *paste.snapshots.borrow()));
    *paste.text.lock().unwrap() = SECOND;
    paste.send(Command::Resume).await;
    paste.reach(Some(false)).await;
    seen.push(format!("{:?}", *paste.snapshots.borrow()));
    paste.send(Command::Done).await;
    paste.reach(None).await;
    seen.push(format!("{:?}", *paste.snapshots.borrow()));
    let spans = paste.control.drain().await;
    assert_eq!(spans.len(), 1);
    seen.push(format!("{spans:?}"));
    assert!(
        paste.nested().contains(&copy(SECRET)) && paste.nested().contains(&copy(SECOND)),
        "the text was never carried, so its absence below proves nothing"
    );
    let calls = paste.calls.lock().unwrap().clone();
    for text in [SECRET, SECOND] {
        assert!(
            calls.iter().all(|call| !call.contains(text)),
            "a command line carried the text"
        );
        assert!(
            seen.iter().all(|record| !record.contains(text)),
            "a snapshot or span carried the text"
        );
    }
    assert!(
        calls.iter().all(|call| call.starts_with("hyprctl ")
            || call.starts_with("wl-paste ")
            || call.starts_with("omarchy-notification-")),
        "nothing but Hyprland, wl-paste and the paused notification may run on the host: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .filter(|call| call.starts_with("wl-paste "))
            .all(|call| !call.contains("--watch") && !call.contains("--primary")),
        "{calls:?}"
    );
    paste.stop().await;
}

#[tokio::test]
async fn a_failing_host_read_leaves_the_takeover_running_and_done_releases() {
    let mut paste = start(TEXT_TYPES, true, Recorder::default());
    paste.send(Command::TakeControl).await;
    paste.reach(Some(false)).await;
    assert!(paste.nested().is_empty(), "{:?}", paste.nested());
    let call = paste.held();
    paste.send(Command::Done).await;
    paste.reach(None).await;
    tokio::time::timeout(std::time::Duration::from_secs(1), call)
        .await
        .expect("Done did not release the held call")
        .unwrap();
    assert_eq!(paste.nested(), [Nested::Clear]);
    paste.stop().await;
}

#[tokio::test]
async fn a_failing_session_write_leaves_the_takeover_running_and_done_releases() {
    let mut paste = start(
        TEXT_TYPES,
        false,
        Recorder {
            copy_fails: true,
            ..Recorder::default()
        },
    );
    paste.send(Command::TakeControl).await;
    paste.reach(Some(false)).await;
    paste.send(Command::StepAway).await;
    paste.reach(Some(true)).await;
    paste.send(Command::Resume).await;
    paste.reach(Some(false)).await;
    let reads = paste
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with("wl-paste --no-newline"))
        .count();
    assert_eq!(reads, 2, "Take control and Resume each read the host text");
    let call = paste.held();
    paste.send(Command::Done).await;
    paste.reach(None).await;
    tokio::time::timeout(std::time::Duration::from_secs(1), call)
        .await
        .expect("Done did not release the held call")
        .unwrap();
    assert_eq!(paste.nested(), [Nested::Clear]);
    paste.stop().await;
}
