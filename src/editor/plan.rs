use crate::timeline::{Millis, SessionTimeline, TimelineEvent};
pub const ACTION_BEFORE: Millis = Millis(300);
pub const ACTION_AFTER: Millis = Millis(1500);
pub const MIN_IDLE: Millis = Millis(1000);
pub const FAST_FORWARD: Millis = Millis(600);
pub use crate::title_card::{
    OUTPUT_FPS, Span, TAKEOVER_GUARD, TITLE_CARD, TitleCard, cut_spans, title_cards,
};
pub struct Margins {
    pub before: Millis,
    pub after: Millis,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentKind {
    RealSpeed,
    TitleCard(TitleCard),
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
        match &self.kind {
            SegmentKind::RealSpeed => {
                (((self.end.0 - self.start.0) as u128 * OUTPUT_FPS as u128 + 500) / 1000).max(1)
                    as u64
            }
            SegmentKind::FastForward { .. } => FAST_FORWARD.0 * OUTPUT_FPS / 1000,
            SegmentKind::TitleCard(_) => TITLE_CARD.0 * OUTPUT_FPS / 1000,
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
            | TimelineEvent::Takeover { .. }
            | TimelineEvent::HelpRequest { .. } => None,
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
    let cards = title_cards(timeline.events());
    for span in excluded {
        let inside: Vec<_> = cards
            .iter()
            .filter(|(card, _)| card.start < span.end && card.end > span.start)
            .collect();
        let mut start = span.start;
        for (index, (_, card)) in inside.iter().enumerate() {
            let end = inside
                .get(index + 1)
                .map_or(span.end, |(next, _)| next.start.max(start));
            segments.push(Segment {
                start,
                end,
                kind: SegmentKind::TitleCard(card.clone()),
            });
            start = end;
        }
    }
    segments.sort_by_key(|segment| segment.start);
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

    #[test]
    fn takeover_and_guard_never_reach_non_card_segments() {
        let timeline = SessionTimeline::parse(r#"{"schema_version":1,"video_lead_ms":0,"events":[{"kind":"takeover","reason":"2FA code","start_ms":1000,"end_ms":4000}]}"#).unwrap();
        let plan = plan_edit(
            &timeline,
            &[],
            Millis(5000),
            &[Span {
                start: Millis(850),
                end: Millis(4150),
            }],
            Margins {
                before: ACTION_BEFORE,
                after: ACTION_AFTER,
            },
        );
        assert_eq!(plan.segments().len(), 3);
        assert_eq!(plan.segments()[1].frames(), 60);
        assert_eq!(plan.segments()[0].end, Millis(850));
        assert_eq!(plan.segments()[2].start, Millis(4150));
        assert!(plan.video_time(Millis(2000)) >= Millis(850));
        assert!(plan.video_time(Millis(2000)) < Millis(2850));
    }

    #[test]
    fn takeover_guard_is_removed_from_idle_segments_and_later_check_maps_after_card() {
        let mut timeline = SessionTimeline::new(Millis(0));
        timeline.push(TimelineEvent::Takeover {
            reason: None,
            start_ms: Millis(3000),
            end_ms: Millis(5000),
        });
        timeline.push(TimelineEvent::CheckMark {
            check: crate::timeline::CheckId(1),
            outcome: crate::timeline::CheckOutcome::Passed,
            at_ms: Millis(9000),
        });
        let excluded = cut_spans(&timeline, Millis(10000), ACTION_AFTER);
        let plan = plan_edit(
            &timeline,
            &[Span {
                start: Millis(0),
                end: Millis(8000),
            }],
            Millis(10000),
            &excluded,
            Margins {
                before: ACTION_BEFORE,
                after: ACTION_AFTER,
            },
        );
        assert!(
            plan.segments()
                .iter()
                .any(|segment| matches!(segment.kind, SegmentKind::FastForward { .. }))
        );
        for segment in plan.segments() {
            if !matches!(segment.kind, SegmentKind::TitleCard(_)) {
                assert!(
                    segment.end <= Millis(2850) || segment.start >= Millis(5150),
                    "{segment:?}"
                );
            }
        }
        assert_eq!(plan.video_time(Millis(9000)), Millis(4200));
    }
    #[test]
    fn takeover_guards_clamp_to_recording_and_merge_overlaps() {
        let mut timeline = SessionTimeline::new(Millis(0));
        for (start, end) in [(0, 1000), (1200, 2000), (4900, 6000)] {
            timeline.push(TimelineEvent::Takeover {
                reason: None,
                start_ms: Millis(start),
                end_ms: Millis(end),
            });
        }
        assert_eq!(
            cut_spans(&timeline, Millis(5000), ACTION_AFTER),
            vec![
                Span {
                    start: Millis(0),
                    end: Millis(2150)
                },
                Span {
                    start: Millis(4750),
                    end: Millis(5000)
                }
            ]
        );
    }

    #[test]
    fn merged_guards_keep_a_title_card_for_each_takeover() {
        let mut timeline = SessionTimeline::new(Millis(0));
        for (start, end, reason) in [(1000, 2000, "first"), (2200, 4200, "second")] {
            timeline.push(TimelineEvent::Takeover {
                start_ms: Millis(start),
                end_ms: Millis(end),
                reason: Some(reason.into()),
            });
        }
        let plan = plan_edit(
            &timeline,
            &[],
            Millis(5000),
            &cut_spans(&timeline, Millis(5000), ACTION_AFTER),
            Margins {
                before: ACTION_BEFORE,
                after: ACTION_AFTER,
            },
        );
        let cards: Vec<_> = plan
            .segments()
            .iter()
            .filter_map(|s| match &s.kind {
                SegmentKind::TitleCard(card) => Some(card.text()),
                SegmentKind::RealSpeed | SegmentKind::FastForward { .. } => None,
            })
            .collect();
        assert_eq!(
            cards,
            [
                "You were in control · 0:01 · first",
                "You were in control · 0:02 · second"
            ]
        );
    }

    #[test]
    fn a_help_request_wait_leaves_the_video_even_while_the_screen_moves() {
        let mut timeline = SessionTimeline::new(Millis(0));
        timeline.push(TimelineEvent::Call {
            tool: "click".into(),
            action: true,
            start_ms: Millis(1000),
            end_ms: Millis(1200),
            is_error: false,
        });
        timeline.push(TimelineEvent::HelpRequest {
            reason: "2FA code".into(),
            start_ms: Millis(1500),
            end_ms: Millis(284_000),
            outcome: crate::timeline::HelpOutcome::Answered,
        });
        timeline.push(TimelineEvent::Takeover {
            reason: Some("2FA code".into()),
            start_ms: Millis(284_000),
            end_ms: Millis(306_000),
        });
        let plan = plan_edit(
            &timeline,
            &[],
            Millis(310_000),
            &cut_spans(&timeline, Millis(310_000), ACTION_AFTER),
            Margins {
                before: ACTION_BEFORE,
                after: ACTION_AFTER,
            },
        );

        let shape: Vec<_> = plan
            .segments()
            .iter()
            .map(|s| match &s.kind {
                SegmentKind::TitleCard(card) => (s.start.0, s.end.0, card.text()),
                SegmentKind::RealSpeed | SegmentKind::FastForward { .. } => {
                    (s.start.0, s.end.0, String::new())
                }
            })
            .collect();
        assert_eq!(
            shape,
            [
                (0, 2700, String::new()),
                (
                    2700,
                    306_150,
                    "Waited 4:42 · You were in control 0:22 · 2FA code".into()
                ),
                (306_150, 310_000, String::new()),
            ]
        );
    }
}
