use crate::timeline::{Millis, SessionTimeline, TimelineEvent};
pub const ACTION_BEFORE: Millis = Millis(300);
pub const ACTION_AFTER: Millis = Millis(1500);
pub const MIN_IDLE: Millis = Millis(1000);
pub const FAST_FORWARD: Millis = Millis(600);
pub const OUTPUT_FPS: u64 = 30;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: Millis,
    pub end: Millis,
}
pub struct Margins {
    pub before: Millis,
    pub after: Millis,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    RealSpeed,
    FastForward { real_ms: Millis },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub start: Millis,
    pub end: Millis,
    pub kind: SegmentKind,
}
impl Segment {
    pub fn frames(&self) -> u64 {
        match self.kind {
            SegmentKind::RealSpeed => {
                (((self.end.0 - self.start.0) as u128 * OUTPUT_FPS as u128 + 500) / 1000).max(1)
                    as u64
            }
            SegmentKind::FastForward { .. } => FAST_FORWARD.0 * OUTPUT_FPS / 1000,
        }
    }
}
pub struct EditPlan {
    segments: Vec<Segment>,
}
impl EditPlan {
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
    pub fn output_length(&self) -> Millis {
        Millis(self.segments.iter().map(Segment::frames).sum::<u64>() * 1000 / OUTPUT_FPS)
    }
}
pub fn plan_edit(
    timeline: &SessionTimeline,
    frozen: &[Span],
    duration: Millis,
    excluded: &[Span],
    margins: Margins,
) -> EditPlan {
    let windows: Vec<_> = timeline
        .events()
        .iter()
        .filter_map(|event| match event {
            TimelineEvent::Call {
                action: true,
                start_ms,
                end_ms,
                ..
            } => Some(Span {
                start: Millis(start_ms.0.saturating_sub(margins.before.0).min(duration.0)),
                end: Millis(end_ms.0.saturating_add(margins.after.0).min(duration.0)),
            }),
            TimelineEvent::Call { action: false, .. }
            | TimelineEvent::CheckMark { .. }
            | TimelineEvent::Takeover { .. } => None,
        })
        .collect();
    let mut edges = vec![Millis(0), duration];
    for span in windows.iter().chain(frozen).chain(excluded) {
        edges.extend([
            Millis(span.start.0.min(duration.0)),
            Millis(span.end.0.min(duration.0)),
        ]);
    }
    edges.sort_unstable();
    edges.dedup();
    let contains = |spans: &[Span], t: Millis| spans.iter().any(|s| s.start <= t && t < s.end);
    let mut pieces: Vec<Segment> = Vec::new();
    for pair in edges.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if contains(excluded, start) {
            continue;
        }
        let idle = contains(frozen, start) && !contains(&windows, start);
        let kind = if idle {
            SegmentKind::FastForward {
                real_ms: Millis(end.0 - start.0),
            }
        } else {
            SegmentKind::RealSpeed
        };
        if let Some(last) = pieces.last_mut()
            && last.end == start
            && matches!(
                (&last.kind, &kind),
                (SegmentKind::RealSpeed, SegmentKind::RealSpeed)
                    | (
                        SegmentKind::FastForward { .. },
                        SegmentKind::FastForward { .. }
                    )
            )
        {
            last.end = end;
            if idle {
                last.kind = SegmentKind::FastForward {
                    real_ms: Millis(end.0 - last.start.0),
                };
            }
        } else {
            pieces.push(Segment { start, end, kind });
        }
    }
    let mut segments: Vec<Segment> = Vec::new();
    for mut piece in pieces {
        if piece.end.0 - piece.start.0 < MIN_IDLE.0 {
            piece.kind = SegmentKind::RealSpeed;
        }
        if let Some(last) = segments.last_mut()
            && last.end == piece.start
            && last.kind == SegmentKind::RealSpeed
            && piece.kind == SegmentKind::RealSpeed
        {
            last.end = piece.end;
        } else {
            segments.push(piece);
        }
    }
    EditPlan { segments }
}
impl crate::report::VideoTimeMap for EditPlan {
    fn video_time(&self, raw: Millis) -> Millis {
        let mut frames = 0u64;
        for s in &self.segments {
            if raw < s.start {
                return Millis(frames * 1000 / OUTPUT_FPS);
            }
            let n = s.frames();
            if raw < s.end {
                let numerator = frames as u128 * (s.end.0 - s.start.0) as u128
                    + n as u128 * (raw.0 - s.start.0) as u128;
                return Millis(
                    (numerator * 1000 / OUTPUT_FPS as u128 / (s.end.0 - s.start.0) as u128) as u64,
                );
            }
            frames += n;
        }
        self.output_length()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::VideoTimeMap;
    use crate::timeline::TimelineEvent;
    fn plan(
        actions: &[(u64, u64, bool)],
        frozen: &[(u64, u64)],
        excluded: &[(u64, u64)],
        duration: u64,
    ) -> EditPlan {
        let mut t = SessionTimeline::new(Millis(0));
        for &(start, end, action) in actions {
            t.push(TimelineEvent::Call {
                tool: "click".into(),
                action,
                start_ms: Millis(start),
                end_ms: Millis(end),
                is_error: true,
            });
        }
        let spans = |xs: &[(u64, u64)]| {
            xs.iter()
                .map(|&(a, b)| Span {
                    start: Millis(a),
                    end: Millis(b),
                })
                .collect::<Vec<_>>()
        };
        plan_edit(
            &t,
            &spans(frozen),
            Millis(duration),
            &spans(excluded),
            Margins {
                before: Millis(300),
                after: Millis(1500),
            },
        )
    }
    fn shape(p: &EditPlan) -> Vec<(u64, u64, bool)> {
        p.segments()
            .iter()
            .map(|s| {
                (
                    s.start.0,
                    s.end.0,
                    matches!(s.kind, SegmentKind::FastForward { .. }),
                )
            })
            .collect()
    }
    #[test]
    fn overlapping_action_windows_merge() {
        assert_eq!(
            shape(&plan(
                &[(1000, 1300, true), (2000, 2300, true)],
                &[(0, 6000)],
                &[],
                6000
            )),
            vec![(0, 3800, false), (3800, 6000, true)]
        );
    }
    #[test]
    fn an_action_at_zero_clamps_its_window() {
        assert_eq!(
            shape(&plan(&[(0, 100, true)], &[(0, 5000)], &[], 5000)),
            vec![(0, 1600, false), (1600, 5000, true)]
        );
    }
    #[test]
    fn an_action_at_the_end_clamps_its_window() {
        for end in [5000, 6000] {
            assert_eq!(
                shape(&plan(&[(4900, end, true)], &[(0, 5000)], &[], 5000)),
                vec![(0, 4600, true), (4600, 5000, false)]
            );
        }
    }
    #[test]
    fn a_moving_screen_stays_at_real_speed() {
        assert_eq!(shape(&plan(&[], &[], &[], 5000)), vec![(0, 5000, false)]);
    }
    #[test]
    fn a_short_idle_stretch_stays_at_real_speed() {
        assert_eq!(
            shape(&plan(&[], &[(100, 999)], &[], 5000)),
            vec![(0, 5000, false)]
        );
    }
    #[test]
    fn a_read_only_call_makes_no_window() {
        assert_eq!(
            shape(&plan(&[(1000, 2000, false)], &[(0, 5000)], &[], 5000)),
            vec![(0, 5000, true)]
        );
    }
    #[test]
    fn an_excluded_span_is_removed() {
        let p = plan(&[], &[(0, 5000)], &[(700, 4500)], 5000);
        assert_eq!(shape(&p), vec![(0, 700, false), (4500, 5000, false)]);
        assert_eq!(p.video_time(Millis(1000)), Millis(700));
    }
    #[test]
    fn no_actions_allow_the_whole_frozen_recording_to_fast_forward() {
        assert_eq!(
            shape(&plan(&[], &[(0, 5000)], &[], 5000)),
            vec![(0, 5000, true)]
        );
    }
    #[test]
    fn the_worked_shape_has_frame_derived_length() {
        let p = plan(
            &[(1000, 1300, true), (6000, 6300, true)],
            &[(8000, 18000)],
            &[],
            20000,
        );
        assert_eq!(
            shape(&p),
            vec![(0, 8000, false), (8000, 18000, true), (18000, 20000, false)]
        );
        assert_eq!(p.output_length(), Millis(10600));
        assert_eq!(p.segments()[1].frames(), 18);
        assert_eq!(
            p.segments()[1].kind,
            SegmentKind::FastForward {
                real_ms: Millis(10000)
            }
        );
    }
    #[test]
    fn the_time_map_is_linear_through_a_fast_forward() {
        let p = plan(&[], &[(8000, 18000)], &[], 20000);
        for (raw, out) in [(8000, 8000), (13000, 8300), (18000, 8600), (25000, 10600)] {
            assert_eq!(p.video_time(Millis(raw)), Millis(out));
        }
    }
    #[test]
    fn tiny_real_speed_segments_keep_one_frame() {
        let p = plan(&[], &[], &[], 1);
        assert_eq!(p.segments()[0].frames(), 1);
        assert_eq!(p.output_length(), Millis(33));
    }
}
