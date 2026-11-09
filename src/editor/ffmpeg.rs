// Every read of the Raw Recording forces the mov demuxer. The sandbox can write the session folder,
// and a planted playlist would otherwise make host ffmpeg open other files.
use super::plan::{FAST_FORWARD, Segment, SegmentKind, Span, TAKEOVER_SHOWN};
use crate::timeline::Millis;
use std::path::Path;

pub fn parse_freezes(stderr: &str, duration: Millis) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start = None;
    for line in stderr.lines() {
        let parse = |key: &str| {
            line.split_once(key)
                .and_then(|(_, v)| v.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| Millis(((v * 1000.0).round() as u64).min(duration.0)))
        };
        if let Some(t) = parse("lavfi.freezedetect.freeze_start:") {
            start = Some(t);
        }
        if let Some(end) = parse("lavfi.freezedetect.freeze_end:")
            && let Some(begin) = start.take()
            && begin < end
        {
            spans.push(Span { start: begin, end });
        }
    }
    if let Some(start) = start
        && start < duration
    {
        spans.push(Span {
            start,
            end: duration,
        });
    }
    spans
}
fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
fn seconds(ms: Millis) -> String {
    format!("{:.3}", ms.0 as f64 / 1000.0)
}
fn common() -> Vec<String> {
    [
        "-nostdin",
        "-hide_banner",
        "-nostats",
        "-y",
        "-progress",
        "pipe:1",
    ]
    .map(str::to_owned)
    .to_vec()
}
// Only the freezedetect job needs ffmpeg's info log, and an error message shouldn't carry its banner.
fn quiet() -> Vec<String> {
    crate::title_card::ffmpeg_prefix()
}
pub fn duration_args(raw: &Path) -> Vec<String> {
    [
        "-v",
        "error",
        "-f",
        "mov",
        "-show_entries",
        "format=duration",
        "-of",
        "csv=p=0",
    ]
    .map(str::to_owned)
    .into_iter()
    .chain([path(raw)])
    .collect()
}
pub fn freeze_args(raw: &Path) -> Vec<String> {
    let mut args = common();
    args.extend([
        "-f".into(),
        "mov".into(),
        "-i".into(),
        path(raw),
        "-vf".into(),
        // -55 dB lets a caret blink, Cua's cursor and small looping animations count as
        // still, while typing and a new message still count as change (`docs/facts.md`).
        "freezedetect=n=-55dB:d=0.5".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]);
    args
}
use crate::title_card::encode_args;
/// The colour of the Takeover label, picked so it reads on the blurred footage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTone {
    Light,
    Dark,
}
impl TextTone {
    /// Light footage (mean luma at least half of 255) gets dark text. Unknown footage gets light
    /// text.
    pub fn for_luma(luma: Option<f64>) -> Self {
        if luma.is_some_and(|luma| luma >= 128.0) {
            Self::Dark
        } else {
            Self::Light
        }
    }
    fn color(self) -> &'static str {
        match self {
            Self::Light => "0xeee2cc",
            Self::Dark => "0x193e58",
        }
    }
}
/// Measures a segment's mean brightness at two small frames a second.
pub fn luma_args(raw: &Path, segment: &Segment) -> Vec<String> {
    let mut args = common();
    args.extend([
        "-ss".into(),
        seconds(segment.start),
        "-t".into(),
        seconds(Millis(segment.end.0 - segment.start.0)),
        "-f".into(),
        "mov".into(),
        "-i".into(),
        path(raw),
        "-an".into(),
        "-vf".into(),
        "fps=2,scale=64:-2,signalstats,metadata=print:key=lavfi.signalstats.YAVG".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]);
    args
}
pub fn parse_luma(stderr: &str) -> Option<f64> {
    let values: Vec<f64> = stderr
        .lines()
        .filter_map(|line| line.split_once("lavfi.signalstats.YAVG="))
        .filter_map(|(_, value)| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .collect();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}
// The blur must leave typed digits unreadable.
fn takeover_sigma(height: u32) -> u32 {
    (height as f64 / 24.0).round().max(1.0) as u32
}
pub fn segment_args(
    raw: &Path,
    segment: &Segment,
    frames: u64,
    size: (u32, u32),
    font: &str,
    tone: TextTone,
    out: &Path,
) -> Vec<String> {
    let mut args = quiet();
    args.extend([
        "-ss".into(),
        seconds(segment.start),
        "-t".into(),
        seconds(Millis(segment.end.0 - segment.start.0)),
        "-f".into(),
        "mov".into(),
        "-i".into(),
        path(raw),
        "-an".into(),
    ]);
    // Cua Driver records full range yuvj420p, and format=yuv420p alone keeps that range.
    let filter = match &segment.kind {
        SegmentKind::RealSpeed => {
            "fps=30:start_time=0,tpad=stop_mode=clone:stop_duration=1,scale=out_range=tv,format=yuv420p".into()
        }
        SegmentKind::Takeover { real_ms } => {
            let text = "You were in control";
            let fontsize = (size.1 as f64 / 28.0)
                .min(size.0 as f64 * 0.9 / (0.6 * text.chars().count() as f64));
            format!(
                "fps=30:start_time=0,setpts=PTS*{:.12},fps=30,tpad=stop_mode=clone:stop_duration=1,gblur=sigma={},drawtext=fontfile={}:text={text}:expansion=none:fontcolor={}:fontsize={fontsize}:x=(w-text_w)/2:y=(h-text_h)/2,scale=out_range=tv,format=yuv420p",
                TAKEOVER_SHOWN.0 as f64 / real_ms.0 as f64,
                takeover_sigma(size.1),
                crate::title_card::escape_font(font),
                tone.color(),
            )
        }
        SegmentKind::FastForward { real_ms } => format!(
            "fps=30:start_time=0,setpts=PTS*{:.12},fps=30,tpad=stop_mode=clone:stop_duration=1,scale=out_range=tv,format=yuv420p",
            FAST_FORWARD.0 as f64 / real_ms.0 as f64
        ),
    };
    args.extend(["-vf".into(), filter]);
    args.extend(encode_args(frames, out));
    args
}
pub fn join_args(list: &Path, out: &Path) -> Vec<String> {
    let mut args = quiet();
    args.extend([
        "-f".into(),
        "concat".into(),
        "-i".into(),
        path(list),
        "-c".into(),
        "copy".into(),
        "-movflags".into(),
        "+faststart".into(),
        path(out),
    ]);
    args
}
pub fn size_args(raw: &Path) -> Vec<String> {
    [
        "-v",
        "error",
        "-f",
        "mov",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height",
        "-of",
        "csv=p=0:s=x",
    ]
    .map(str::to_owned)
    .into_iter()
    .chain([path(raw)])
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const CAPTURED: &str =
        "[Parsed_freezedetect_0 @ 0x7fc9bc003440] lavfi.freezedetect.freeze_start: 8
[Parsed_freezedetect_0 @ 0x7fc9bc003440] lavfi.freezedetect.freeze_duration: 4
[Parsed_freezedetect_0 @ 0x7fc9bc003440] lavfi.freezedetect.freeze_end: 12";
    #[test]
    fn captured_freezes_have_start_and_end_times() {
        assert_eq!(
            parse_freezes(CAPTURED, Millis(20000)),
            vec![Span {
                start: Millis(8000),
                end: Millis(12000)
            }]
        );
    }
    #[test]
    fn an_unclosed_freeze_runs_to_the_recording_end() {
        assert_eq!(
            parse_freezes(
                "[Parsed_freezedetect_0 @ 0x7f2b90003440] lavfi.freezedetect.freeze_start: 3",
                Millis(5000)
            ),
            vec![Span {
                start: Millis(3000),
                end: Millis(5000)
            }]
        );
    }
    #[test]
    fn fractional_freezes_ignore_bad_values_and_clamp_to_duration() {
        assert_eq!(
            parse_freezes(
                "unrelated: 7\nlavfi.freezedetect.freeze_start: invalid\nlavfi.freezedetect.freeze_start: 1.966667\nlavfi.freezedetect.freeze_end: NaN\nlavfi.freezedetect.freeze_end: 99",
                Millis(5000)
            ),
            vec![Span {
                start: Millis(1967),
                end: Millis(5000)
            }]
        );
    }
    #[test]
    fn font_paths_survive_both_escaping_levels() {
        assert_eq!(
            crate::title_card::escape_font(r"a\b'c:d,e[f];g"),
            r"a\\\\b\\\'c\\:d\,e\[f\]\;g"
        );
    }
    #[test]
    fn dark_footage_gets_light_text_and_light_footage_gets_dark_text() {
        let luma = |values: &str| parse_luma(values);
        assert_eq!(
            TextTone::for_luma(luma(
                "x lavfi.signalstats.YAVG=30.5\ny lavfi.signalstats.YAVG=41.5"
            )),
            TextTone::Light
        );
        assert_eq!(
            TextTone::for_luma(luma(
                "lavfi.signalstats.YAVG=200\nlavfi.signalstats.YAVG=nan"
            )),
            TextTone::Dark
        );
        assert_eq!(TextTone::for_luma(luma("no stats")), TextTone::Light);
    }
    #[test]
    fn fast_forward_segments_compress_without_drawing_text() {
        let segment = Segment {
            start: Millis(0),
            end: Millis(18000),
            kind: SegmentKind::FastForward {
                real_ms: Millis(18000),
            },
        };
        let args = segment_args(
            Path::new("raw"),
            &segment,
            18,
            (0, 0),
            "",
            TextTone::Light,
            Path::new("part"),
        );
        let filter = args.windows(2).find(|v| v[0] == "-vf").unwrap();
        assert!(filter[1].contains("setpts=PTS*0.033333333333"));
        assert!(!filter[1].contains("drawtext"));
    }
    #[test]
    fn raw_reads_force_mov_and_segments_have_exact_frames() {
        let s = Segment {
            start: Millis(123),
            end: Millis(1123),
            kind: SegmentKind::RealSpeed,
        };
        let a = segment_args(
            Path::new("raw"),
            &s,
            30,
            (0, 0),
            "",
            TextTone::Light,
            Path::new("part"),
        );
        assert!(a.windows(4).any(|v| v == ["-f", "mov", "-i", "raw"]));
        assert!(a.windows(2).any(|v| v == ["-frames:v", "30"]));
        assert!(a.iter().position(|v| v == "-ss") < a.iter().position(|v| v == "-i"));
        assert!(
            a.iter()
                .any(|v| v.contains("tpad=stop_mode=clone:stop_duration=1"))
        );
        assert!(
            freeze_args(Path::new("raw"))
                .windows(4)
                .any(|v| v == ["-f", "mov", "-i", "raw"])
        );
        assert_eq!(
            duration_args(Path::new("raw")),
            [
                "-v",
                "error",
                "-f",
                "mov",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
                "raw"
            ]
        );
    }
}
