// This stopgap editor is removed by #11.
mod ffmpeg;
mod plan;
pub use plan::*;

use crate::process::{ProcessError, ProcessRunner};
use crate::timeline::{Millis, SessionTimeline};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error(
        "Cannot edit the Verification Video because the Raw Recording is missing or is not a regular file. Check the recording and try again."
    )]
    NoRecording,
    #[error("Cannot edit the Verification Video: {0}")]
    Process(#[from] ProcessError),
    #[error(
        "Cannot read the Raw Recording duration: ffprobe printed {0:?}, not a positive number. Check the recording with ffprobe and try again."
    )]
    Duration(String),
    #[error(
        "Cannot find a font for the video labels because fc-match returned no path. Install JetBrains Mono and try again."
    )]
    Font,
    #[error("Cannot read the Raw Recording size: {0}. Check it with ffprobe and retry.")]
    Size(String),
    #[error(
        "Cannot prepare or save the Verification Video: {0}. Check the session folder permissions and available space, then try again."
    )]
    Filesystem(#[from] std::io::Error),
    #[error("Cannot finish the Verification Video filesystem task: {0}. Try the edit again.")]
    Task(#[from] tokio::task::JoinError),
}
#[derive(Debug, Clone, Copy)]
pub struct EditProgress {
    pub percent: crate::ipc::Percent,
    pub segment_index: usize,
    pub segment_count: usize,
    pub out_time_us: u64,
}
async fn job(
    runner: &dyn ProcessRunner,
    args: &[String],
    index: usize,
    count: usize,
    progress: &mut (dyn FnMut(EditProgress) + Send),
) -> Result<String, EditError> {
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    Ok(runner
        .stream("ffmpeg", &args, &mut |line| {
            if let Some(us) = line
                .strip_prefix("out_time_us=")
                .and_then(|v| v.parse().ok())
            {
                progress(EditProgress {
                    percent: crate::ipc::Percent(0),
                    segment_index: index,
                    segment_count: count,
                    out_time_us: us,
                });
            }
        })
        .await?)
}
pub async fn edit_incomplete(
    runner: &dyn ProcessRunner,
    raw: &Path,
    timeline: &SessionTimeline,
    out: &Path,
    progress: &mut (dyn FnMut(EditProgress) + Send),
    reason: &crate::checklist::IncompleteReason,
) -> Result<EditPlan, EditError> {
    edit_ending(runner, raw, timeline, out, progress, Some(reason)).await
}
pub async fn edit(
    runner: &dyn ProcessRunner,
    raw: &Path,
    timeline: &SessionTimeline,
    out: &Path,
    on_progress: &mut (dyn FnMut(EditProgress) + Send),
) -> Result<EditPlan, EditError> {
    edit_ending(runner, raw, timeline, out, on_progress, None).await
}
async fn edit_ending(
    runner: &dyn ProcessRunner,
    raw: &Path,
    timeline: &SessionTimeline,
    out: &Path,
    on_progress: &mut (dyn FnMut(EditProgress) + Send),
    incomplete: Option<&crate::checklist::IncompleteReason>,
) -> Result<EditPlan, EditError> {
    let destination = out.to_owned();
    // The sandbox can plant verification.mp4; remove it before any failure can let Evidence file it.
    tokio::task::spawn_blocking(move || match std::fs::remove_file(destination) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    })
    .await??;
    let duration = recording_duration(runner, raw).await?;
    let freezes = job(runner, &ffmpeg::freeze_args(raw), 0, 0, &mut |_| {}).await?;
    let plan = plan_edit(
        timeline,
        &ffmpeg::parse_freezes(&freezes, duration),
        duration,
        &cut_spans(timeline, duration, ACTION_AFTER),
        Margins {
            before: ACTION_BEFORE,
            after: ACTION_AFTER,
        },
    );
    let font = if incomplete.is_some()
        || plan
            .segments()
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::Takeover { .. }))
    {
        let matched = runner
            .output(
                "fc-match",
                &["-f", "%{family}|%{file}", "JetBrains Mono"],
                None,
            )
            .await?;
        let (family, file) = matched.split_once('|').ok_or(EditError::Font)?;
        if file.trim().is_empty() {
            return Err(EditError::Font);
        }
        // fc-match returns its closest font even when JetBrains Mono is missing.
        if !family.replace(' ', "").contains("JetBrainsMono") {
            tracing::warn!(family, "Video labels use a fallback font");
        }
        file.trim().to_owned()
    } else {
        String::new()
    };
    let size = if incomplete.is_some()
        || plan
            .segments()
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::Takeover { .. }))
    {
        let args = ffmpeg::size_args(raw);
        let refs: Vec<_> = args.iter().map(String::as_str).collect();
        let text = runner.output("ffprobe", &refs, None).await?;
        text.trim()
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
            .filter(|(w, h)| *w > 0 && *h > 0)
            .ok_or(EditError::Size(text))?
    } else {
        (0, 0)
    };
    let mut name = out.as_os_str().to_owned();
    name.push(".parts");
    let parts = std::path::PathBuf::from(name);
    let directory = parts.clone();
    tokio::task::spawn_blocking(move || std::fs::create_dir(directory)).await??;
    let mut whole = progress::Progress::new(&plan, incomplete);
    let mut report_progress = |mut p: EditProgress| {
        if let Some(percent) = whole.update(p.segment_index, p.out_time_us) {
            p.percent = percent;
            on_progress(p);
        }
    };
    let result = async {
        for (i, segment) in plan.segments().iter().enumerate() {
            let output = parts.join(format!("{i:04}.mp4"));
            let tone = match segment.kind {
                SegmentKind::Takeover { .. } => {
                    let luma =
                        job(runner, &ffmpeg::luma_args(raw, segment), i, 0, &mut |_| {}).await?;
                    ffmpeg::TextTone::for_luma(ffmpeg::parse_luma(&luma))
                }
                SegmentKind::RealSpeed | SegmentKind::FastForward { .. } => ffmpeg::TextTone::Light,
            };
            let args =
                ffmpeg::segment_args(raw, segment, segment.frames(), size, &font, tone, &output);
            job(
                runner,
                &args,
                i,
                plan.segments().len(),
                &mut report_progress,
            )
            .await?;
        }
        if let Some(reason) = incomplete {
            let output = parts.join(format!("{:04}.mp4", plan.segments().len()));
            let args = crate::title_card::text_card_args(
                &format!("Incomplete · {}", reason.short()),
                size,
                &font,
                &output,
            );
            job(
                runner,
                &args,
                plan.segments().len(),
                plan.segments().len() + 1,
                &mut report_progress,
            )
            .await?;
        }
        let list = parts.join("list.txt");
        let content = (0..plan.segments().len() + usize::from(incomplete.is_some()))
            .map(|i| format!("file '{i:04}.mp4'\n"))
            .collect::<String>();
        let list_path = list.clone();
        tokio::task::spawn_blocking(move || std::fs::write(list_path, content)).await??;
        let joined = parts.join("joined.mp4");
        job(
            runner,
            &ffmpeg::join_args(&list, &joined),
            plan.segments().len(),
            plan.segments().len(),
            &mut |_| {},
        )
        .await?;
        let destination = out.to_owned();
        tokio::task::spawn_blocking(move || std::fs::rename(joined, destination)).await??;
        Ok::<(), EditError>(())
    }
    .await;
    let cleanup = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(parts)).await?;
    result?;
    // The video is already in place, and Evidence removes the session folder when it files.
    if let Err(error) = cleanup {
        tracing::warn!(%error, "could not remove the editor's parts folder");
    }
    Ok(plan)
}

pub async fn recording_duration(
    runner: &dyn ProcessRunner,
    raw: &Path,
) -> Result<Millis, EditError> {
    let source = raw.to_owned();
    let regular = tokio::task::spawn_blocking(move || {
        std::fs::symlink_metadata(source).is_ok_and(|m| m.is_file())
    })
    .await?;
    if !regular {
        return Err(EditError::NoRecording);
    }
    let args = ffmpeg::duration_args(raw);
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    let duration_text = runner.output("ffprobe", &args, None).await?;
    let duration = duration_text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| Millis((v * 1000.0).round() as u64))
        .filter(|v| v.0 > 0)
        .ok_or_else(|| EditError::Duration(duration_text))?;
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ProcessFuture;
    struct Runner;
    impl ProcessRunner for Runner {
        fn output<'a>(
            &'a self,
            _: &'a str,
            _: &'a [&'a str],
            _: Option<&'a Path>,
        ) -> ProcessFuture<'a> {
            Box::pin(async { Ok("2".into()) })
        }
        fn stream<'a>(
            &'a self,
            _: &'a str,
            args: &'a [&'a str],
            stdout: &'a mut (dyn FnMut(&str) + Send),
        ) -> ProcessFuture<'a> {
            Box::pin(async move {
                for line in ["out_time_ms=99", "out_time_us=bad", "out_time_us=123"] {
                    stdout(line);
                }
                if !args.contains(&"null") {
                    std::fs::write(args.last().unwrap(), b"video").unwrap();
                }
                Ok(String::new())
            })
        }
    }
    #[tokio::test]
    async fn progress_reports_only_segment_stdout_times() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.mp4");
        let out = dir.path().join("verification.mp4");
        std::fs::write(&raw, b"raw").unwrap();
        let mut progress = Vec::new();
        edit(
            &Runner,
            &raw,
            &SessionTimeline::new(Millis(0)),
            &out,
            &mut |p| progress.push(p),
        )
        .await
        .unwrap();
        assert_eq!(progress.len(), 1);
        assert_eq!(
            (
                progress[0].segment_index,
                progress[0].segment_count,
                progress[0].out_time_us
            ),
            (0, 1, 123)
        );
        assert!(out.is_file());
        assert!(!dir.path().join("verification.mp4.parts").exists());
    }
}

#[cfg(test)]
mod takeover_tests {
    use super::*;
    use crate::process::ProcessFuture;
    use std::sync::Mutex;
    struct Runner(Mutex<Vec<Vec<String>>>, Mutex<String>);
    impl ProcessRunner for Runner {
        fn output<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            _: Option<&'a Path>,
        ) -> ProcessFuture<'a> {
            Box::pin(async move {
                Ok(if program == "fc-match" {
                    "JetBrains Mono|/fonts/mono.ttf"
                } else if args.contains(&"stream=width,height") {
                    "640x480"
                } else {
                    "5"
                }
                .into())
            })
        }
        fn stream<'a>(
            &'a self,
            _: &'a str,
            args: &'a [&'a str],
            _: &'a mut (dyn FnMut(&str) + Send),
        ) -> ProcessFuture<'a> {
            Box::pin(async move {
                if let Some(index) = args.iter().position(|arg| *arg == "concat") {
                    *self.1.lock().unwrap() = std::fs::read_to_string(args[index + 2]).unwrap();
                }
                self.0
                    .lock()
                    .unwrap()
                    .push(args.iter().map(|s| (*s).to_owned()).collect());
                if !args.contains(&"null") {
                    std::fs::write(args.last().unwrap(), b"video").unwrap();
                }
                Ok(if args.iter().any(|arg| arg.contains("signalstats")) {
                    "lavfi.signalstats.YAVG=221.5\nlavfi.signalstats.YAVG=218.5\n".into()
                } else {
                    String::new()
                })
            })
        }
    }
    #[tokio::test]
    async fn a_takeover_blurs_raw_footage_before_drawing_only_its_label_in_a_tone_that_reads() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.mp4");
        std::fs::write(&raw, b"raw").unwrap();
        let mut timeline = SessionTimeline::new(Millis(0));
        timeline.push(crate::timeline::TimelineEvent::Takeover {
            reason: Some("2FA code".into()),
            start_ms: Millis(1000),
            end_ms: Millis(4000),
        });
        let runner = Runner(Mutex::new(Vec::new()), Mutex::new(String::new()));
        let plan = edit(
            &runner,
            &raw,
            &timeline,
            &dir.path().join("verification.mp4"),
            &mut |_| {},
        )
        .await
        .unwrap();
        assert_eq!(plan.segments().len(), 3);
        let calls = runner.0.lock().unwrap();
        let probe = &calls[2];
        assert!(probe.windows(2).any(|v| v == ["-ss", "0.850"]));
        assert!(probe.iter().any(|arg| arg.contains("signalstats")));
        let takeover = &calls[3];
        assert!(
            takeover
                .windows(4)
                .any(|v| v == ["-f", "mov", "-i", raw.to_str().unwrap()])
        );
        assert!(takeover.windows(2).any(|v| v == ["-ss", "0.850"]));
        assert!(takeover.windows(2).any(|v| v == ["-t", "3.300"]));
        assert!(takeover.windows(2).any(|v| v == ["-frames:v", "60"]));
        let filter = &takeover.windows(2).find(|v| v[0] == "-vf").unwrap()[1];
        assert!(filter.find("gblur").unwrap() < filter.find("drawtext").unwrap());
        assert!(filter.contains("gblur=sigma=20"));
        assert!(filter.contains("setpts=PTS*0.606060606061"));
        assert_eq!(filter.matches("drawtext=").count(), 1);
        assert_eq!(
            filter
                .split(":text=")
                .nth(1)
                .unwrap()
                .split(':')
                .next()
                .unwrap(),
            "You were in control"
        );
        assert!(filter.contains("fontsize=17.142857142857142"));
        assert!(
            filter.contains("fontcolor=0x193e58"),
            "light footage gets dark text"
        );
        assert!(!filter.contains("box="));
        assert!(filter.contains("x=(w-text_w)/2:y=(h-text_h)/2"));
    }
    #[tokio::test]
    async fn incomplete_video_appends_a_card_after_the_last_segment() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.mp4");
        std::fs::write(&raw, b"raw").unwrap();
        let runner = Runner(Mutex::new(Vec::new()), Mutex::new(String::new()));
        edit_incomplete(
            &runner,
            &raw,
            &SessionTimeline::new(Millis(0)),
            &dir.path().join("verification.mp4"),
            &mut |_| {},
            &crate::checklist::IncompleteReason::DesktopStopped,
        )
        .await
        .unwrap();
        let calls = runner.0.lock().unwrap();
        let card = &calls[calls.len() - 2];
        assert!(
            card.iter()
                .any(|arg| arg.contains("Incomplete · Desktop stopped"))
        );
        assert!(card.last().unwrap().ends_with("0001.mp4"));
        assert!(runner.1.lock().unwrap().contains("file '0001.mp4'"));
    }
}

mod progress;
