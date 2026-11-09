use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, ensure};
use keek::timeline::{Millis, SessionTimeline, TimelineEvent};
use serde_json::Value;
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    raw: PathBuf,
    timeline: PathBuf,
    out: PathBuf,
}

impl Fixture {
    fn new(freeze_start: f64) -> Result<Self> {
        Self::generate(
            "20",
            &format!(
                "[0:v]split[screen][still];[screen][still]freezeframes=first={}:last=269:replace={}[video]",
                (freeze_start * 15.0) as u64,
                (freeze_start * 15.0) as u64
            ),
            &[(1000, 1300), (6000, 6300)],
        )
    }

    fn generate(duration: &str, filter: &str, actions: &[(u64, u64)]) -> Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("keek-live-editor-")
            .tempdir()?;
        let raw = directory.path().join("raw.mp4");
        let timeline = directory.path().join("timeline.json");
        let out = directory.path().join("verification.mp4");
        run(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc=size=1280x800:rate=15:duration={duration}"),
                "-filter_complex_threads",
                "1",
                "-filter_complex",
                filter,
                "-map",
                "[video]",
                "-c:v",
                "libx264",
                "-threads",
                "1",
                "-pix_fmt",
                "yuvj420p",
                "-y",
            ])
            .arg(&raw))?;
        let mut events = SessionTimeline::new(Millis(0));
        for &(start, end) in actions {
            events.push(TimelineEvent::Call {
                tool: "click".into(),
                action: true,
                start_ms: Millis(start),
                end_ms: Millis(end),
                is_error: false,
            });
        }
        std::fs::write(&timeline, serde_json::to_vec(&events)?)?;
        Ok(Self {
            directory,
            raw,
            timeline,
            out,
        })
    }

    fn edit_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_keek"));
        command
            .arg("edit")
            .arg(&self.raw)
            .arg(&self.timeline)
            .arg(&self.out);
        command
    }

    fn edit(&self) -> Result<Value> {
        let output = run(&mut self.edit_command())?;
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}

fn run(command: &mut Command) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("run {command:?}"))?;
    ensure!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn probe(path: &Path) -> Result<Value> {
    let output = run(Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path))?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn duration(path: &Path) -> Result<f64> {
    probe(path)?["format"]["duration"]
        .as_str()
        .context("missing duration")?
        .parse()
        .context("invalid duration")
}

fn segments(edit: &Value) -> Result<&[Value]> {
    Ok(edit["segments"].as_array().context("missing segments")?)
}

fn number(segment: &Value, key: &str) -> Result<u64> {
    segment[key]
        .as_u64()
        .with_context(|| format!("missing {key}: {segment}"))
}

#[test]
#[ignore = "requires ffmpeg, ffprobe and JetBrains Mono"]
fn live_editor_output_matches_the_plan_length() -> Result<()> {
    let fixture = Fixture::new(8.0)?;
    let edit = fixture.edit()?;
    let seconds = duration(&fixture.out)?;
    println!(
        "Verification Video duration={seconds}s, editor output_ms={}",
        edit["output_ms"]
    );
    ensure!(
        (seconds - 10.6).abs() <= 0.2,
        "expected 10.6s, found {seconds}s"
    );
    ensure!(
        (edit["output_ms"].as_f64().context("missing output_ms")? / 1000.0 - seconds).abs() <= 0.2,
        "editor duration does not match ffprobe"
    );
    Ok(())
}

#[test]
#[ignore = "requires ffmpeg, ffprobe and JetBrains Mono"]
fn live_editor_real_speed_segments_match_the_raw_video() -> Result<()> {
    let fixture = Fixture::new(8.0)?;
    let edit = fixture.edit()?;
    let mut output_frame = 0;
    for (index, segment) in segments(&edit)?.iter().enumerate() {
        let frames = number(segment, "frames")?;
        if segment["kind"] == "real_speed" {
            let start = number(segment, "start_ms")? as f64 / 1000.0;
            let end = number(segment, "end_ms")? as f64 / 1000.0;
            let stats = fixture.directory.path().join(format!("ssim-{index}.log"));
            let filter = format!(
                "[0:v]trim=start={start}:end={end},setpts=PTS-STARTPTS,fps=30[raw];[1:v]trim=start_frame={output_frame}:end_frame={},setpts=PTS-STARTPTS[edited];[raw][edited]ssim=stats_file={}",
                output_frame + frames,
                stats.display()
            );
            run(Command::new("ffmpeg")
                .args(["-v", "error", "-nostdin", "-i"])
                .arg(&fixture.raw)
                .arg("-i")
                .arg(&fixture.out)
                .args([
                    "-filter_complex_threads",
                    "1",
                    "-filter_complex",
                    &filter,
                    "-f",
                    "null",
                    "-",
                ]))?;
            let stats = std::fs::read_to_string(stats)?;
            let scores: Vec<f64> = stats
                .lines()
                .map(|line| -> Result<f64> {
                    line.split_whitespace()
                        .find_map(|part| part.strip_prefix("All:"))
                        .context("missing per-frame SSIM All")?
                        .parse()
                        .context("invalid SSIM")
                })
                .collect::<Result<_>>()?;
            ensure!(
                scores.len() >= frames as usize,
                "SSIM omitted real-speed frames: {} < {frames}",
                scores.len()
            );
            let minimum = scores.iter().copied().fold(1.0_f64, f64::min);
            println!(
                "real-speed segment {index}: frames={}, minimum SSIM All={minimum}",
                scores.len()
            );
            ensure!(
                minimum > 0.95,
                "real-speed segment {index} minimum SSIM All={minimum}"
            );
        }
        output_frame += frames;
    }
    Ok(())
}

#[test]
#[ignore = "requires ffmpeg, ffprobe and JetBrains Mono"]
fn live_editor_fast_forward_frame_draws_no_label() -> Result<()> {
    let fixture = Fixture::new(8.0)?;
    let edit = fixture.edit()?;
    let mut output_frame = 0;
    let mut label = None;
    for segment in segments(&edit)? {
        let frames = number(segment, "frames")?;
        if segment["kind"] == "fast_forward" {
            label = Some((output_frame + frames / 2, number(segment, "start_ms")?));
            break;
        }
        output_frame += frames;
    }
    let (frame, raw_ms) = label.context("no Fast-forward segment")?;
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/live_editor_frame.png");
    run(Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&fixture.out)
        .args([
            "-vf",
            &format!("select=eq(n\\,{frame})"),
            "-frames:v",
            "1",
            "-threads",
            "1",
            "-y",
        ])
        .arg(&target))?;
    let crop = "crop=400:110:440:650";
    let edited = run(Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&target)
        .args([
            "-vf",
            crop,
            "-frames:v",
            "1",
            "-pix_fmt",
            "gray",
            "-f",
            "rawvideo",
            "-",
        ]))?
    .stdout;
    let raw = run(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-ss",
            &format!("{}", raw_ms as f64 / 1000.0),
            "-i",
        ])
        .arg(&fixture.raw)
        .args([
            "-vf",
            crop,
            "-frames:v",
            "1",
            "-pix_fmt",
            "gray",
            "-f",
            "rawvideo",
            "-",
        ]))?
    .stdout;
    ensure!(
        edited.len() == raw.len() && !edited.is_empty(),
        "label crops have different dimensions"
    );
    let changed = edited
        .iter()
        .zip(raw.iter())
        .filter(|(left, right)| left.abs_diff(**right) > 32)
        .count();
    println!(
        "label crop changed pixels={changed}; frame={}",
        target.display()
    );
    ensure!(
        changed < 50,
        "a Fast-forward frame changed {changed} pixels where a label would sit"
    );
    Ok(())
}

#[test]
#[ignore = "requires ffmpeg, ffprobe and JetBrains Mono"]
fn live_editor_motion_stays_at_real_speed() -> Result<()> {
    let fixture = Fixture::new(15.0)?;
    let edit = fixture.edit()?;
    let seconds = duration(&fixture.out)?;
    println!("motion through 15s: Verification Video duration={seconds}s");
    ensure!(
        (seconds - 17.6).abs() <= 0.2,
        "expected 17.6s, found {seconds}s"
    );
    ensure!(
        (edit["output_ms"].as_f64().context("missing output_ms")? / 1000.0 - seconds).abs() <= 0.2,
        "motion fixture duration does not match the plan"
    );
    Ok(())
}

#[test]
#[ignore = "requires ffmpeg, ffprobe and JetBrains Mono"]
fn live_editor_spike_shaped_recording_lands_near_nine_seconds() -> Result<()> {
    // The original spike's Action times are unavailable, so these stand-ins keep motion in their margins.
    let filter = "[0:v]split=3[moving][base][replacement];[base][replacement]freezeframes=first=0:last=304:replace=0[still];[still][moving]overlay=enable='between(t,0.7,8.3)'[video]";
    let fixture = Fixture::generate(
        "20.3",
        filter,
        &[
            (1000, 1300),
            (2400, 2700),
            (3700, 4000),
            (5200, 5500),
            (6500, 6800),
        ],
    )?;
    fixture.edit()?;
    let seconds = duration(&fixture.out)?;
    println!("20.3s spike stand-in: Verification Video duration={seconds}s");
    ensure!(
        (8.0..=11.0).contains(&seconds),
        "spike duration={seconds}s is outside 8-11s"
    );
    Ok(())
}

#[test]
#[ignore = "requires ffmpeg, ffprobe and JetBrains Mono"]
fn live_editor_output_is_one_constant_rate_stream() -> Result<()> {
    let fixture = Fixture::new(8.0)?;
    fixture.edit()?;
    let metadata = probe(&fixture.out)?;
    let streams = metadata["streams"].as_array().context("missing streams")?;
    ensure!(streams.len() == 1, "expected one stream: {streams:?}");
    let video = &streams[0];
    ensure!(video["codec_type"] == "video", "output stream is not video");
    ensure!(
        video["r_frame_rate"] == "30/1" && video["avg_frame_rate"] == "30/1",
        "output is not CFR 30fps: {video}"
    );
    ensure!(
        video["pix_fmt"] == "yuv420p",
        "output is not yuv420p: {video}"
    );
    println!("output stream: 30/1 CFR, yuv420p");
    Ok(())
}

#[test]
#[ignore = "requires ffmpeg, ffprobe and JetBrains Mono"]
fn live_editor_refuses_a_symlinked_recording() -> Result<()> {
    let fixture = Fixture::new(8.0)?;
    let link = fixture.directory.path().join("linked.mp4");
    std::os::unix::fs::symlink(&fixture.raw, &link)?;
    let output = Command::new(env!("CARGO_BIN_EXE_keek"))
        .arg("edit")
        .arg(link)
        .arg(&fixture.timeline)
        .arg(&fixture.out)
        .output()?;
    ensure!(
        !output.status.success(),
        "editor accepted a symlinked Raw Recording"
    );
    ensure!(
        !fixture.out.exists(),
        "editor left output after refusing a symlink"
    );
    println!(
        "symlink refused: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}
