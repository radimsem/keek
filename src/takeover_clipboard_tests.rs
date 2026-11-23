use super::*;
use crate::process::{BytesFuture, ProcessError, ProcessFuture};
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SECRET: &str = "made-up-recovery-key-7Q2X";

enum Paste {
    Types(&'static str),
    TypesFail(&'static str),
    ContentFails,
}

struct Host {
    calls: Mutex<Vec<Vec<String>>>,
    paste: Paste,
}
impl Host {
    fn new(paste: Paste) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            paste,
        }
    }
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}
impl ProcessRunner for Host {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> ProcessFuture<'a> {
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
        _: Option<&'a Path>,
    ) -> BytesFuture<'a> {
        Box::pin(async move {
            let mut call = vec![program.to_owned()];
            call.extend(args.iter().map(|arg| (*arg).to_owned()));
            self.calls.lock().unwrap().push(call);
            assert_eq!(program, "wl-paste");
            let listing = args.contains(&"--list-types");
            match (&self.paste, listing) {
                (Paste::Types(types), true) => Ok(types.as_bytes().to_vec()),
                (Paste::TypesFail(message), true) => Err(ProcessError::Failed {
                    program: program.into(),
                    message: (*message).into(),
                }),
                (Paste::ContentFails, true) => Ok(b"text/plain;charset=utf-8\n".to_vec()),
                (Paste::ContentFails, false) => Err(ProcessError::Failed {
                    program: program.into(),
                    message: format!("exit status: 1: {SECRET}"),
                }),
                (Paste::Types(_) | Paste::TypesFail(_), false) => Ok(SECRET.as_bytes().to_vec()),
            }
        })
    }
}

#[test]
fn carried_text_debug_shows_only_its_length() {
    let text = CarriedText::new(SECRET.as_bytes().to_vec());
    let debug = format!("{text:?}");
    assert!(!debug.contains(SECRET), "Debug printed the carried text");
    assert!(debug.contains(&SECRET.len().to_string()));
}

#[tokio::test]
async fn plain_text_is_read_exactly_without_an_added_newline() {
    let host = Host::new(Paste::Types(
        "text/html\ntext/plain;charset=utf-8\ntext/plain\nUTF8_STRING\n",
    ));
    let text = read_host_text(&host)
        .await
        .expect("reading the host clipboard failed")
        .expect("text on the host clipboard was skipped");
    assert!(
        text.as_bytes() == SECRET.as_bytes(),
        "the carried text differs from the host's"
    );
    let calls = host.calls();
    assert_eq!(calls[0], ["wl-paste", "--list-types"]);
    let read = &calls[1];
    assert!(read.contains(&"--no-newline".to_owned()), "{read:?}");
    let kind = read
        .iter()
        .position(|arg| arg == "--type")
        .map(|at| read[at + 1].as_str());
    assert!(
        kind.is_some_and(|kind| kind.starts_with("text/plain")),
        "the read must ask for plain text: {read:?}"
    );
    assert!(
        calls.iter().flatten().all(|arg| !arg.contains(SECRET)),
        "a command line carried the text"
    );
}

#[tokio::test]
async fn legacy_text_types_count_as_plain_text() {
    for types in ["UTF8_STRING\n", "STRING\nTEXT\n", "text/plain\n"] {
        let host = Host::new(Paste::Types(types));
        assert!(
            read_host_text(&host).await.unwrap().is_some(),
            "{types:?} was not read"
        );
    }
}

#[tokio::test]
async fn other_types_and_an_empty_clipboard_are_skipped_without_an_error() {
    for paste in [
        Paste::Types("image/png\nimage/jpeg\n"),
        Paste::Types("text/html\ntext/uri-list\n"),
        Paste::Types(""),
        Paste::TypesFail("exit status: 1: Nothing is copied"),
    ] {
        let host = Host::new(paste);
        let result = read_host_text(&host).await;
        assert!(
            matches!(result, Ok(None)),
            "expected a skip, got {result:?}"
        );
        assert_eq!(host.calls(), [["wl-paste", "--list-types"]]);
    }
}

#[tokio::test]
async fn a_failed_read_reports_an_error_without_the_programs_output() {
    let host = Host::new(Paste::TypesFail(
        "exit status: 1: Failed to connect to a Wayland server",
    ));
    assert!(matches!(
        read_host_text(&host).await,
        Err(ClipboardError::HostRead(_))
    ));
    let host = Host::new(Paste::ContentFails);
    let error = read_host_text(&host)
        .await
        .expect_err("a failed content read was not reported");
    assert!(matches!(error, ClipboardError::HostRead(_)));
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains(SECRET),
        "the error repeats what wl-paste printed"
    );
}

fn reader(fifo: &Path) -> (Arc<Mutex<Vec<u8>>>, std::thread::JoinHandle<()>) {
    // Read and write, as the sandbox reader opens it, so this side never sees end of file.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fifo)
        .unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = received.clone();
    let thread = std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            let read = file.read(&mut buffer).unwrap();
            let mut all = sink.lock().unwrap();
            all.extend_from_slice(&buffer[..read]);
            if all.ends_with(b"clear\n") {
                return;
            }
        }
    });
    (received, thread)
}

#[test]
fn the_fifo_is_private_to_the_developer() {
    let directory = tempfile::tempdir().unwrap();
    create_fifo(directory.path()).unwrap();
    let metadata = std::fs::symlink_metadata(directory.path().join(FIFO_NAME)).unwrap();
    assert!(metadata.file_type().is_fifo());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
}

#[tokio::test]
async fn copy_and_clear_reach_the_fifo_as_length_prefixed_frames() {
    let directory = tempfile::tempdir().unwrap();
    create_fifo(directory.path()).unwrap();
    let (received, thread) = reader(&directory.path().join(FIFO_NAME));
    let fifo = SessionFifo::new(directory.path());
    fifo.copy(CarriedText::new(SECRET.as_bytes().to_vec()))
        .await
        .expect("copy failed");
    fifo.clear().await.expect("clear failed");
    tokio::task::spawn_blocking(move || thread.join().unwrap())
        .await
        .unwrap();
    let expected = format!("copy {}\n{SECRET}clear\n", SECRET.len());
    assert!(
        *received.lock().unwrap() == expected.as_bytes(),
        "the frames differ from `copy <length>` and `clear`"
    );
}

#[tokio::test]
async fn a_fifo_without_a_reader_fails_at_once() {
    let directory = tempfile::tempdir().unwrap();
    let fifo = SessionFifo::new(directory.path());
    let started = Instant::now();
    assert!(matches!(fifo.clear().await, Err(ClipboardError::Nested(_))));
    create_fifo(directory.path()).unwrap();
    assert!(matches!(
        fifo.copy(CarriedText::new(SECRET.as_bytes().to_vec()))
            .await,
        Err(ClipboardError::Nested(_))
    ));
    assert!(fifo.clear().await.is_err());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a missing reader blocked the Takeover"
    );
}

fn wait_for(path: &Path) -> String {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if let Ok(text) = std::fs::read_to_string(path)
            && text.ends_with('\n')
        {
            return text;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{} never appeared", path.display());
}

#[tokio::test]
async fn the_sandbox_reader_copies_sensitive_plain_text_and_clears_in_order() {
    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    let out = directory.path().join("out");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    let fake = bin.join("wl-copy");
    std::fs::write(
        &fake,
        "#!/bin/bash\n\
         n=$(ls \"$FAKE_OUT\" | grep -c '\\.args$')\n\
         if [[ \" $* \" != *\" --clear \"* ]]; then cat >\"$FAKE_OUT/$n.stdin\"; fi\n\
         printf '%s\\n' \"$*\" >\"$FAKE_OUT/$n.args\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    create_fifo(directory.path()).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/nested/clipboard.sh");
    let mut child = std::process::Command::new("bash")
        .arg(&script)
        .arg(directory.path().join(FIFO_NAME))
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("FAKE_OUT", &out)
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let fifo = SessionFifo::new(directory.path());
    let started = Instant::now();
    let copied = loop {
        // The reader may not have opened the FIFO yet.
        match fifo
            .copy(CarriedText::new(
                format!("{SECRET}\nsecond line").into_bytes(),
            ))
            .await
        {
            Ok(()) => break true,
            Err(_) if started.elapsed() < Duration::from_secs(10) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(_) => break false,
        }
    };
    assert!(copied, "the sandbox reader never opened the FIFO");
    fifo.clear().await.expect("clear failed");
    let outcome = tokio::task::spawn_blocking({
        let out = out.clone();
        move || (wait_for(&out.join("0.args")), wait_for(&out.join("1.args")))
    })
    .await;
    child.kill().unwrap();
    child.wait().unwrap();
    let (first, second) = outcome.unwrap();
    assert!(first.contains("--sensitive"), "{first:?}");
    assert!(first.contains("--type text/plain"), "{first:?}");
    assert!(
        !first.contains(SECRET),
        "wl-copy got the text as an argument"
    );
    assert!(
        std::fs::read(out.join("0.stdin")).unwrap() == format!("{SECRET}\nsecond line").as_bytes(),
        "wl-copy's standard input differs from the carried text"
    );
    assert_eq!(second.trim(), "--clear");
}
