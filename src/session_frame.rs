use crate::process::ProcessRunner;
use crate::session_folder::SessionName;
use std::path::{Path, PathBuf};
pub async fn capture(
    runner: &dyn ProcessRunner,
    raw: &Path,
    frames: &Path,
    session: &SessionName,
) -> Option<PathBuf> {
    let recording = raw.to_owned();
    if !tokio::task::spawn_blocking(move || {
        std::fs::symlink_metadata(recording).is_ok_and(|m| m.is_file())
    })
    .await
    .unwrap_or(false)
    {
        return None;
    }
    let directory = frames.to_owned();
    let prepared = tokio::task::spawn_blocking(move || {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
    })
    .await;
    match prepared {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, "could not prepare Session Card frames");
            return None;
        }
        Err(error) => {
            tracing::warn!(%error, "could not prepare Session Card frames");
            return None;
        }
    }
    let frame = frames.join(format!("{session}.png"));
    let (Some(raw), Some(destination)) = (raw.to_str(), frame.to_str()) else {
        tracing::warn!("could not capture Session Card frame: path is not UTF-8");
        return None;
    };
    let result = runner
        .output(
            "ffmpeg",
            &[
                // The sandbox can write the session folder and plant a playlist.
                "-f",
                "mov",
                "-sseof",
                "-1",
                "-i",
                raw,
                "-update",
                "1",
                "-y",
                destination,
            ],
            None,
        )
        .await;
    if let Err(error) = result {
        tracing::warn!(%error, "could not capture Session Card frame");
        let partial = frame.clone();
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(partial)).await;
        return None;
    }
    Some(frame)
}
pub async fn clear(runtime: &Path) {
    let frames = runtime.join("frames");
    let result = tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(frames) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    })
    .await;
    if !matches!(result, Ok(Ok(()))) {
        tracing::warn!(?result, "could not clear old Session Card frames");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{ProcessError, ProcessFuture};
    use std::sync::Mutex;
    struct Runner {
        fail: bool,
        calls: Mutex<Vec<Vec<String>>>,
    }
    impl ProcessRunner for Runner {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            _: Option<&'a Path>,
        ) -> ProcessFuture<'a> {
            Box::pin(async move {
                assert_eq!(program, "ffmpeg");
                self.calls
                    .lock()
                    .unwrap()
                    .push(args.iter().map(|s| (*s).to_owned()).collect());
                std::fs::write(args.last().unwrap(), b"frame").unwrap();
                if self.fail {
                    Err(ProcessError::Failed {
                        program: program.into(),
                        message: "bad recording".into(),
                    })
                } else {
                    Ok(String::new())
                }
            })
        }
    }
    #[tokio::test]
    async fn last_frame_forces_mov_and_seeks_from_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.mp4");
        std::fs::write(&raw, b"recording").unwrap();
        let runner = Runner {
            fail: false,
            calls: Mutex::new(vec![]),
        };
        let frame = capture(&runner, &raw, dir.path(), &"test_12-00-00".parse().unwrap())
            .await
            .unwrap();
        assert!(frame.is_file());
        let calls = runner.calls.lock().unwrap();
        assert_eq!(
            calls[0],
            [
                "-f",
                "mov",
                "-sseof",
                "-1",
                "-i",
                raw.to_str().unwrap(),
                "-update",
                "1",
                "-y",
                frame.to_str().unwrap()
            ]
        );
    }
    #[tokio::test]
    async fn failed_capture_removes_the_partial_frame() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.mp4");
        std::fs::write(&raw, b"recording").unwrap();
        let runner = Runner {
            fail: true,
            calls: Mutex::new(vec![]),
        };
        assert!(
            capture(&runner, &raw, dir.path(), &"test".parse().unwrap())
                .await
                .is_none()
        );
        assert!(!dir.path().join("test.png").exists());
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn frames_are_private_and_old_frames_are_cleared() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        std::fs::create_dir(&frames).unwrap();
        std::fs::write(frames.join("old.png"), b"old").unwrap();
        clear(dir.path()).await;
        assert!(!frames.exists());
        std::fs::create_dir(&frames).unwrap();
        std::fs::set_permissions(&frames, std::fs::Permissions::from_mode(0o755)).unwrap();
        let raw = dir.path().join("raw.mp4");
        std::fs::write(&raw, b"recording").unwrap();
        let runner = Runner {
            fail: false,
            calls: Mutex::new(vec![]),
        };
        capture(&runner, &raw, &frames, &"test".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(
            std::fs::metadata(&frames).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    #[tokio::test]
    async fn non_file_recording_does_not_run_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("recording.mp4");
        std::fs::create_dir(&raw).unwrap();
        let runner = Runner {
            fail: false,
            calls: Mutex::new(vec![]),
        };
        assert!(
            capture(&runner, &raw, dir.path(), &"test".parse().unwrap())
                .await
                .is_none()
        );
        assert!(runner.calls.lock().unwrap().is_empty());
    }
}
