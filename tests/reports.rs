use keek::checklist::{Checklist, SessionEnd, Verdict};
use keek::report::{self, IdentityTimeMap, ReportInputs, VideoTimeMap};
use keek::session::Revision;
use keek::timeline::{CheckId, CheckOutcome, Millis};

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
            SessionEnd::Incomplete {
                reason: "Verifier disconnected.".into(),
            },
        ),
    ] {
        let mut list = Checklist::new(vec!["Parser accepts BOM".into()]);
        list.mark(CheckId(1), outcome, "Observed input".into(), Millis(62000))
            .unwrap();
        let added = list.add("Parser keeps text".into()).unwrap();
        if name != "incomplete" {
            list.mark(added, CheckOutcome::Passed, "".into(), Millis(125000))
                .unwrap();
        }
        let revision = Revision {
            branch: Some("fix/17-parser-bom".into()),
            commit: "a1b2c3d456".into(),
            dirty: true,
        };
        let rendered = report::render(&ReportInputs {
            session: "myapp_14-03-22",
            goal: "Verify parser BOM",
            revision: &revision,
            summary: "Checked both inputs.",
            checks: list.checks(),
            end: &end,
            time_map: &IdentityTimeMap,
            video_error: None,
        });
        let golden = std::fs::read_to_string(format!("tests/fixtures/reports/{name}.md")).unwrap();
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
        session: "s",
        goal: "g",
        revision: &revision,
        summary: " ",
        checks: list.checks(),
        end: &SessionEnd::Verdict(Verdict::Pass),
        time_map: &Map,
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
        &SessionEnd::Incomplete {
            reason: "exit".into(),
        },
        "goal\nnext\rline",
        &[("report", "2026-10-30/report.md".into())],
    );
    assert_eq!(
        line,
        "2026-10-30 14:03:22 | detached HEAD | é | Incomplete | goal next line | [report](2026-10-30/report.md)\n"
    );
}
