use crate::timeline::{Millis, SessionTimeline, TimelineEvent};
use std::path::Path;
pub const TAKEOVER_GUARD: Millis = Millis(150);
pub const TAKEOVER_CARD: Millis = Millis(2000);
pub const OUTPUT_FPS: u64 = 30;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: Millis,
    pub end: Millis,
}
pub fn takeover_spans(timeline: &SessionTimeline, duration: Millis) -> Vec<Span> {
    let mut spans: Vec<_> = timeline
        .events()
        .iter()
        .filter_map(|event| match event {
            TimelineEvent::Takeover {
                start_ms, end_ms, ..
            } => Some(Span {
                start: Millis(start_ms.0.saturating_sub(TAKEOVER_GUARD.0).min(duration.0)),
                end: Millis(end_ms.0.saturating_add(TAKEOVER_GUARD.0).min(duration.0)),
            }),
            TimelineEvent::Call { .. } | TimelineEvent::CheckMark { .. } => None,
        })
        .filter(|span| span.start < span.end)
        .collect();
    spans.sort_by_key(|span| span.start);
    let mut merged: Vec<Span> = Vec::new();
    for span in spans {
        if let Some(last) = merged.last_mut()
            && span.start <= last.end
        {
            last.end = last.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    merged
}

pub fn escape_font(font: &str) -> String {
    fn escape(text: &str, special: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            if special.contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }
    escape(&escape(font, "\\':"), "\\'[],;")
}

pub fn ffmpeg_prefix() -> Vec<String> {
    [
        "-nostdin",
        "-hide_banner",
        "-nostats",
        "-y",
        "-progress",
        "pipe:1",
        "-v",
        "error",
    ]
    .map(str::to_owned)
    .to_vec()
}
pub fn encode_args(frames: u64, out: &Path) -> Vec<String> {
    vec![
        "-frames:v".into(),
        frames.to_string(),
        "-r".into(),
        OUTPUT_FPS.to_string(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "18".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-video_track_timescale".into(),
        "30000".into(),
        out.to_string_lossy().into_owned(),
    ]
}
pub fn card_args(
    real_ms: Millis,
    reason: Option<&str>,
    size: (u32, u32),
    font: &str,
    out: &Path,
) -> Vec<String> {
    let seconds = real_ms.0 / 1000;
    let mut text = format!("You were in control · {}:{:02}", seconds / 60, seconds % 60);
    if let Some(reason) = reason {
        text.push_str(" · ");
        text.push_str(&reason.replace(['\r', '\n'], " "));
    }
    text_card_args(&text, size, font, out)
}
pub fn text_card_args(text: &str, size: (u32, u32), font: &str, out: &Path) -> Vec<String> {
    let fontsize =
        (size.1 as f64 / 28.0).min(size.0 as f64 * 0.9 / (0.6 * text.chars().count() as f64));
    let mut args = ffmpeg_prefix();
    args.extend([
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("color=c=0x193e58:s={}x{}:r={OUTPUT_FPS}", size.0, size.1),
        "-an".into(),
        "-vf".into(),
        format!(
            "drawtext=fontfile={}:text={}:expansion=none:fontcolor=0xeee2cc:fontsize={fontsize}:x=(w-text_w)/2:y=(h-text_h)/2,format=yuv420p",
            escape_font(font),
            escape_font(text)
        ),
    ]);
    args.extend(encode_args(TAKEOVER_CARD.0 * OUTPUT_FPS / 1000, out));
    args
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_title_fits_a_small_frame() {
        let args = card_args(
            Millis(1000),
            Some(&"x".repeat(40)),
            (640, 480),
            "font",
            Path::new("card.mp4"),
        );
        let filter = args.iter().find(|arg| arg.contains("drawtext")).unwrap();
        let size: f64 = filter
            .split("fontsize=")
            .nth(1)
            .unwrap()
            .split(':')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let chars = "You were in control · 0:01 · ".chars().count() + 40;
        assert!(size <= 480.0 / 28.0 && size * 0.6 * chars as f64 <= 640.0 * 0.9);
    }
    #[test]
    fn title_card_arguments_carry_the_copy_and_probed_size() {
        let args = card_args(
            Millis(42000),
            Some("2FA code"),
            (1280, 800),
            "/fonts/JetBrainsMono.ttf",
            Path::new("part.mp4"),
        );
        assert!(
            args.iter()
                .any(|arg| arg.contains("color=c=0x193e58:s=1280x800:r=30"))
        );
        assert!(args.iter().any(
            |arg| arg.contains(r"You were in control · 0\\:42 · 2FA code")
                && arg.contains("0xeee2cc")
        ));
        assert!(args.windows(2).any(|a| a == ["-frames:v", "60"]));
        assert!(args.windows(2).any(|a| a == ["-c:v", "libx264"]));
        assert!(args.windows(2).any(|a| a == ["-r", "30"]));
        assert!(args.windows(2).any(|a| a == ["-pix_fmt", "yuv420p"]));
        assert!(
            args.windows(2)
                .any(|a| a == ["-video_track_timescale", "30000"])
        );
    }
}
