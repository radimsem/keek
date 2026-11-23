use super::*;
use crate::process::{ProcessError, ProcessFuture, ProcessRunner};
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn name(text: &str) -> SessionName {
    text.parse().unwrap()
}

#[test]
fn every_rail_session_is_due_at_once_then_again_after_the_interval() {
    let mut schedule = StillSchedule::default();
    let start = Instant::now();
    let rail = [name("a"), name("b")];
    assert_eq!(schedule.due(start, &rail), vec![name("a"), name("b")]);
    assert_eq!(
        schedule.due(start + STILL_INTERVAL - Duration::from_millis(1), &rail),
        Vec::<SessionName>::new()
    );
    assert_eq!(
        schedule.due(start + STILL_INTERVAL, &rail),
        vec![name("a"), name("b")]
    );
}

#[test]
fn a_session_back_in_the_rail_gets_a_fresh_still_at_once() {
    let mut schedule = StillSchedule::default();
    let start = Instant::now();
    assert_eq!(schedule.due(start, &[name("a")]), vec![name("a")]);
    assert_eq!(
        schedule.due(start + Duration::from_secs(1), &[]),
        Vec::<SessionName>::new()
    );
    assert_eq!(
        schedule.due(start + Duration::from_secs(2), &[name("a")]),
        vec![name("a")]
    );
}

#[test]
fn a_still_lives_in_the_frames_folder_not_the_session_folder() {
    let frames = std::path::Path::new("/frames");
    assert_eq!(
        still_path(frames, &name("app-12-30-04")),
        frames.join("app-12-30-04-still.jpg")
    );
}

struct Grim {
    calls: Mutex<Vec<Vec<String>>>,
    clients: &'static str,
}
impl ProcessRunner for Grim {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        _: Option<&'a std::path::Path>,
    ) -> ProcessFuture<'a> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(
                std::iter::once(program.to_owned())
                    .chain(args.iter().map(|arg| (*arg).to_owned()))
                    .collect(),
            );
            match program {
                "hyprctl" => Ok(self.clients.into()),
                "grim" => {
                    std::fs::write(args.last().unwrap(), b"jpeg").unwrap();
                    Ok(String::new())
                }
                _ => Err(ProcessError::Failed {
                    program: program.into(),
                    message: "unexpected".into(),
                }),
            }
        })
    }
}

#[tokio::test]
async fn a_still_is_grim_on_the_host_window_by_its_stable_id() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app-still.jpg");
    let grim = Grim {
        calls: Mutex::new(Vec::new()),
        clients: r#"[{"address":"0xdef","stableId":"99"},{"address":"0xabc","stableId":"1a2b"}]"#,
    };
    take(
        &grim,
        &crate::compositor::WindowAddress("0xabc".into()),
        0.197,
        &path,
        Arc::new(oneshot::channel().0),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"jpeg");
    let calls = grim.calls.lock().unwrap();
    assert_eq!(calls[0], ["hyprctl", "clients", "-j"]);
    let still = &calls[1];
    assert_eq!(
        &still[..9],
        [
            "grim", "-T", "1a2b", "-t", "jpeg", "-q", "70", "-s", "0.197"
        ]
    );
    // grim writes beside the still and keek renames it, so the viewer never reads half a file.
    assert_ne!(std::path::Path::new(&still[9]), path);
    assert_eq!(std::path::Path::new(&still[9]).parent(), path.parent());
    assert!(!std::path::Path::new(&still[9]).exists());
}

#[tokio::test]
async fn a_window_missing_from_the_host_takes_no_still() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app-still.jpg");
    let grim = Grim {
        calls: Mutex::new(Vec::new()),
        clients: r#"[{"address":"0xdef","stableId":"99"}]"#,
    };
    let error = take(
        &grim,
        &crate::compositor::WindowAddress("0xabc".into()),
        0.2,
        &path,
        Arc::new(oneshot::channel().0),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("0xabc"), "{error}");
    assert!(!path.exists());
    assert_eq!(grim.calls.lock().unwrap().len(), 1);
}
