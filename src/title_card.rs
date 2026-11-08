use crate::timeline::{HelpOutcome, Millis, SessionTimeline, TimelineEvent};
use std::path::Path;
pub const TAKEOVER_GUARD: Millis = Millis(150);
pub const TITLE_CARD: Millis = Millis(2000);
pub const OUTPUT_FPS: u64 = 30;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: Millis,
    pub end: Millis,
}
/// What the Verification Video shows in place of a Help Request's wait or a Takeover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleCard {
    /// A Takeover the developer started without a Help Request.
    Takeover {
        in_control: Millis,
        reason: Option<String>,
    },
    Answered {
        waited: Millis,
        in_control: Millis,
        reason: String,
    },
    Withdrawn {
        waited: Millis,
        reason: String,
    },
}
impl TitleCard {
    #[must_use]
    pub fn text(&self) -> String {
        let reason = |reason: &str| format!(" · {}", reason.replace(['\r', '\n'], " "));
        match self {
            Self::Takeover {
                in_control,
                reason: r,
            } => format!(
                "You were in control · {}{}",
                clock(*in_control),
                r.as_deref().map(reason).unwrap_or_default()
            ),
            Self::Answered {
                waited,
                in_control,
                reason: r,
            } => format!(
                "Waited {} · You were in control {}{}",
                clock(*waited),
                clock(*in_control),
                reason(r)
            ),
            Self::Withdrawn { waited, reason: r } => format!(
                "Waited {} · Help Request withdrawn{}",
                clock(*waited),
                reason(r)
            ),
        }
    }
    /// The real time the card stands for.
    #[must_use]
    pub fn real_ms(&self) -> Millis {
        match self {
            Self::Takeover { in_control, .. } => *in_control,
            Self::Answered {
                waited, in_control, ..
            } => Millis(waited.0 + in_control.0),
            Self::Withdrawn { waited, .. } => *waited,
        }
    }
}
pub fn clock(time: Millis) -> String {
    let seconds = time.0 / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
/// Each title card with the raw span it stands for, in order. An answered Help Request and
/// the Takeover that starts at its end share one card.
pub fn title_cards(events: &[TimelineEvent]) -> Vec<(Span, TitleCard)> {
    let mut answered = Vec::new();
    let mut cards: Vec<(Span, TitleCard)> = events
        .iter()
        .filter_map(|event| match event {
            TimelineEvent::HelpRequest {
                reason,
                start_ms,
                end_ms,
                outcome,
            } => {
                let waited = Millis(end_ms.0.saturating_sub(start_ms.0));
                let takeover = (*outcome == HelpOutcome::Answered)
                    .then(|| {
                        events.iter().find_map(|event| match event {
                            TimelineEvent::Takeover {
                                start_ms: taken,
                                end_ms: done,
                                ..
                            } if taken == end_ms => Some(*done),
                            TimelineEvent::Takeover { .. }
                            | TimelineEvent::Call { .. }
                            | TimelineEvent::CheckMark { .. }
                            | TimelineEvent::HelpRequest { .. } => None,
                        })
                    })
                    .flatten();
                Some(match takeover {
                    Some(done) => {
                        answered.push(*end_ms);
                        (
                            Span {
                                start: *start_ms,
                                end: done,
                            },
                            TitleCard::Answered {
                                waited,
                                in_control: Millis(done.0.saturating_sub(end_ms.0)),
                                reason: reason.clone(),
                            },
                        )
                    }
                    None => (
                        Span {
                            start: *start_ms,
                            end: *end_ms,
                        },
                        TitleCard::Withdrawn {
                            waited,
                            reason: reason.clone(),
                        },
                    ),
                })
            }
            TimelineEvent::Call { .. }
            | TimelineEvent::CheckMark { .. }
            | TimelineEvent::Takeover { .. } => None,
        })
        .collect();
    cards.extend(events.iter().filter_map(|event| match event {
        TimelineEvent::Takeover {
            reason,
            start_ms,
            end_ms,
        } if !answered.contains(start_ms) => Some((
            Span {
                start: *start_ms,
                end: *end_ms,
            },
            TitleCard::Takeover {
                in_control: Millis(end_ms.0.saturating_sub(start_ms.0)),
                reason: reason.clone(),
            },
        )),
        TimelineEvent::Takeover { .. }
        | TimelineEvent::Call { .. }
        | TimelineEvent::CheckMark { .. }
        | TimelineEvent::HelpRequest { .. } => None,
    }));
    cards.sort_by_key(|(span, _)| span.start);
    cards
}
/// The raw spans the Verification Video leaves out: each Takeover with its guard, and each
/// Help Request's wait from the later of its start and the end of the last Action window
/// (`after` past an Action) before it.
pub fn cut_spans(timeline: &SessionTimeline, duration: Millis, after: Millis) -> Vec<Span> {
    let events = timeline.events();
    let mut spans: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            TimelineEvent::Takeover {
                start_ms, end_ms, ..
            } => Some(Span {
                start: Millis(start_ms.0.saturating_sub(TAKEOVER_GUARD.0)),
                end: Millis(end_ms.0.saturating_add(TAKEOVER_GUARD.0)),
            }),
            TimelineEvent::HelpRequest {
                start_ms, end_ms, ..
            } => {
                let shown = events
                    .iter()
                    .filter_map(|event| match event {
                        TimelineEvent::Call {
                            action: true,
                            start_ms: action,
                            end_ms: done,
                            ..
                        } if action < start_ms => Some(Millis(done.0.saturating_add(after.0))),
                        TimelineEvent::Call { .. }
                        | TimelineEvent::CheckMark { .. }
                        | TimelineEvent::Takeover { .. }
                        | TimelineEvent::HelpRequest { .. } => None,
                    })
                    .max()
                    .unwrap_or(Millis(0));
                Some(Span {
                    start: (*start_ms).max(shown),
                    end: *end_ms,
                })
            }
            TimelineEvent::Call { .. } | TimelineEvent::CheckMark { .. } => None,
        })
        .map(|span| Span {
            start: Millis(span.start.0.min(duration.0)),
            end: Millis(span.end.0.min(duration.0)),
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
pub fn card_args(card: &TitleCard, size: (u32, u32), font: &str, out: &Path) -> Vec<String> {
    text_card_args(&card.text(), size, font, out)
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
    args.extend(encode_args(TITLE_CARD.0 * OUTPUT_FPS / 1000, out));
    args
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_title_fits_a_small_frame() {
        let card = TitleCard::Takeover {
            in_control: Millis(1000),
            reason: Some("x".repeat(40)),
        };
        let args = card_args(&card, (640, 480), "font", Path::new("card.mp4"));
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
        let card = TitleCard::Takeover {
            in_control: Millis(42000),
            reason: Some("2FA code".into()),
        };
        let args = card_args(
            &card,
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
    fn help(start: u64, end: u64, outcome: HelpOutcome) -> TimelineEvent {
        TimelineEvent::HelpRequest {
            reason: "2FA code".into(),
            start_ms: Millis(start),
            end_ms: Millis(end),
            outcome,
        }
    }
    fn takeover(start: u64, end: u64) -> TimelineEvent {
        TimelineEvent::Takeover {
            reason: Some("2FA code".into()),
            start_ms: Millis(start),
            end_ms: Millis(end),
        }
    }
    fn action(start: u64, end: u64) -> TimelineEvent {
        TimelineEvent::Call {
            tool: "click".into(),
            action: true,
            start_ms: Millis(start),
            end_ms: Millis(end),
            is_error: false,
        }
    }
    fn timeline(events: Vec<TimelineEvent>) -> SessionTimeline {
        let mut timeline = SessionTimeline::new(Millis(0));
        for event in events {
            timeline.push(event);
        }
        timeline
    }
    #[test]
    fn an_answered_help_request_shares_one_card_with_its_takeover() {
        let t = timeline(vec![
            help(10_000, 293_000, HelpOutcome::Answered),
            takeover(293_000, 315_000),
        ]);

        let cards = title_cards(t.events());

        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].1.text(),
            "Waited 4:43 · You were in control 0:22 · 2FA code"
        );
        assert_eq!(cards[0].1.real_ms(), Millis(305_000));
    }
    #[test]
    fn a_withdrawn_help_request_gets_its_own_card() {
        let t = timeline(vec![help(10_000, 293_000, HelpOutcome::Withdrawn)]);

        let cards = title_cards(t.events());

        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].1.text(),
            "Waited 4:43 · Help Request withdrawn · 2FA code"
        );
    }
    #[test]
    fn a_takeover_without_a_help_request_keeps_its_own_card() {
        let t = timeline(vec![
            help(1_000, 2_000, HelpOutcome::Withdrawn),
            TimelineEvent::Takeover {
                reason: None,
                start_ms: Millis(5_000),
                end_ms: Millis(47_000),
            },
        ]);

        let texts: Vec<_> = title_cards(t.events())
            .iter()
            .map(|(_, c)| c.text())
            .collect();

        assert_eq!(
            texts,
            [
                "Waited 0:01 · Help Request withdrawn · 2FA code",
                "You were in control · 0:42"
            ]
        );
    }
    #[test]
    fn a_help_request_wait_is_cut_from_the_end_of_the_last_action_window() {
        let t = timeline(vec![
            action(8_000, 9_000),
            help(9_500, 293_000, HelpOutcome::Answered),
            takeover(293_000, 315_000),
        ]);

        let spans = cut_spans(&t, Millis(400_000), Millis(1_500));

        assert_eq!(
            spans,
            [Span {
                start: Millis(10_500),
                end: Millis(315_150),
            }]
        );
    }
    #[test]
    fn a_withdrawn_wait_is_cut_without_a_guard() {
        let t = timeline(vec![help(20_000, 80_000, HelpOutcome::Withdrawn)]);

        let spans = cut_spans(&t, Millis(100_000), Millis(1_500));

        assert_eq!(
            spans,
            [Span {
                start: Millis(20_000),
                end: Millis(80_000),
            }]
        );
    }
}
