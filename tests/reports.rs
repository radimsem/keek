use keek::checklist::{Checklist, SessionEnd, Verdict};
use keek::report::{self, IdentityTimeMap, ReportInputs, VideoTimeMap};
use keek::revision::Revision;
use keek::timeline::{CheckId, CheckOutcome, Millis};

#[test]
fn failed_video_notice_covers_takeover_times_without_double_punctuation() {
    let events = [keek::timeline::TimelineEvent::Takeover {
        reason: Some("2FA code.".into()),
        start_ms: Millis(1000),
        end_ms: Millis(2000),
    }];
    let revision = Revision {
        branch: None,
        commit: "abc".into(),
        dirty: false,
    };
    let rendered = report::render(&ReportInputs {
        session: "s",
        goal: "g",
        revision: &revision,
        summary: "",
        timeline_events: &events,
        checks: &[],
        end: &SessionEnd::Verdict(Verdict::Pass),
        time_map: &IdentityTimeMap,
        actions_error: None,
        video_error: Some("failed"),
    });
    assert!(rendered.contains("2FA code.\n"));
    assert!(!rendered.contains("2FA code.."));
    assert!(rendered.contains("Check and Takeover times"));
}

#[test]
fn reports_match_the_reviewed_markdown() {
    for (name, outcome, end) in [
        (
            "pass",
            CheckOutcome::Passed,
            SessionEnd::Verdict(Verdict::Pass),
        ),
        (
            "needs-fixes",
            CheckOutcome::Failed,
            SessionEnd::Verdict(Verdict::NeedsFixes),
        ),
        (
            "partly-verified",
            CheckOutcome::Skipped,
            SessionEnd::Verdict(Verdict::PartlyVerified),
        ),
        (
            "incomplete",
            CheckOutcome::Passed,
            SessionEnd::Incomplete(keek::checklist::IncompleteReason::OpenerLeft),
        ),
        (
            "idle-minutes",
            CheckOutcome::Passed,
            SessionEnd::Incomplete(keek::checklist::IncompleteReason::Idle {
                limit: std::time::Duration::from_secs(300),
            }),
        ),
        (
            "idle-seconds",
            CheckOutcome::Passed,
            SessionEnd::Incomplete(keek::checklist::IncompleteReason::Idle {
                limit: std::time::Duration::from_secs(3),
            }),
        ),
        (
            "idle-fractional-seconds",
            CheckOutcome::Passed,
            SessionEnd::Incomplete(keek::checklist::IncompleteReason::Idle {
                limit: std::time::Duration::from_millis(1500),
            }),
        ),
        (
            "host-window",
            CheckOutcome::Passed,
            SessionEnd::Incomplete(keek::checklist::IncompleteReason::HostWindowOpened),
        ),
        (
            "desktop-stopped",
            CheckOutcome::Passed,
            SessionEnd::Incomplete(keek::checklist::IncompleteReason::DesktopStopped),
        ),
    ] {
        let mut list = Checklist::new(vec!["Parser accepts BOM".into()]);
        list.mark(CheckId(1), outcome, "Observed input".into(), Millis(62000))
            .unwrap();
        let added = list.add("Parser keeps text".into()).unwrap();
        if matches!(end, SessionEnd::Verdict(_)) {
            list.mark(added, CheckOutcome::Passed, "".into(), Millis(125000))
                .unwrap();
        }
        let revision = Revision {
            branch: Some("fix/17-parser-bom".into()),
            commit: "a1b2c3d456".into(),
            dirty: true,
        };
        let rendered = report::render(&ReportInputs {
            timeline_events: &[],
            session: "myapp_14-03-22",
            goal: "Verify parser BOM",
            revision: &revision,
            summary: "Checked both inputs.",
            checks: list.checks(),
            end: &end,
            time_map: &IdentityTimeMap,
            actions_error: None,
            video_error: None,
        });
        let (fixture, sentence) = match name {
            "idle-minutes" => (
                "incomplete",
                Some("keek ended the session after 5 minutes without a tool call."),
            ),
            "idle-seconds" => (
                "incomplete",
                Some("keek ended the session after 3 seconds without a tool call."),
            ),
            "idle-fractional-seconds" => (
                "incomplete",
                Some("keek ended the session after 1.5 seconds without a tool call."),
            ),
            "host-window" => (
                "incomplete",
                Some(
                    "keek ended the session because the session opened a window on the developer's desktop other than its Session Window.",
                ),
            ),
            "desktop-stopped" => (
                "incomplete",
                Some("The session's desktop stopped before close_session."),
            ),
            _ => (name, None),
        };
        let mut golden =
            std::fs::read_to_string(format!("tests/fixtures/reports/{fixture}.md")).unwrap();
        if let Some(sentence) = sentence {
            golden = golden.replace(
                "keek ended the session because the coding agent's keek mcp disconnected.",
                sentence,
            );
        }
        assert_eq!(rendered, golden);
        assert!(!golden.contains(['\u{2013}', '\u{2014}']));
        assert!(!golden.contains("## Verdict\n\nFail"));
    }
}
#[test]
fn reports_map_video_times_and_handle_detached_short_revisions() {
    struct Map;
    impl VideoTimeMap for Map {
        fn video_time(&self, _: Millis) -> Millis {
            Millis(9000)
        }
    }
    let mut list = Checklist::new(vec!["criterion".into()]);
    list.mark(CheckId(1), CheckOutcome::Passed, "".into(), Millis(60000))
        .unwrap();
    let revision = Revision {
        branch: None,
        commit: "é".into(),
        dirty: false,
    };
    let text = report::render(&ReportInputs {
        timeline_events: &[],
        session: "s",
        goal: "g",
        revision: &revision,
        summary: " ",
        checks: list.checks(),
        end: &SessionEnd::Verdict(Verdict::Pass),
        time_map: &Map,
        actions_error: None,
        video_error: None,
    });
    assert!(text.contains("Verified detached HEAD at é\n"));
    assert!(text.contains("Passed at 0:09"));
    assert!(!text.contains("old observation"));
    assert!(!text.contains("Verifier summary"));
    let line = report::index_line(
        "2026-10-30",
        "14:03:22",
        &revision,
        &SessionEnd::Incomplete(keek::checklist::IncompleteReason::DesktopStopped),
        "goal\nnext\rline",
        &[("report", "2026-10-30/report.md".into())],
    );
    assert_eq!(
        line,
        "2026-10-30 14:03:22 | detached HEAD | é | Incomplete | goal next line | [report](2026-10-30/report.md)\n"
    );
}

#[test]
fn reports_list_takeovers_after_the_checklist_at_mapped_video_times() {
    struct Map;
    impl VideoTimeMap for Map {
        fn video_time(&self, raw: Millis) -> Millis {
            Millis(raw.0 / 2)
        }
    }
    let timeline = keek::timeline::SessionTimeline::parse(r#"{"schema_version":1,"video_lead_ms":0,"events":[{"kind":"takeover","reason":"2FA code","start_ms":130000,"end_ms":172000},{"kind":"takeover","start_ms":180000,"end_ms":181000}]}"#).unwrap();
    let revision = Revision {
        branch: Some("main".into()),
        commit: "abcdef123".into(),
        dirty: false,
    };
    let rendered = report::render(&ReportInputs {
        session: "app",
        goal: "Verify login",
        revision: &revision,
        summary: "",
        checks: &[],
        timeline_events: timeline.events(),
        end: &SessionEnd::Verdict(Verdict::Pass),
        time_map: &Map,
        actions_error: None,
        video_error: None,
    });
    assert_eq!(rendered, include_str!("fixtures/reports/takeover.md"));
}
