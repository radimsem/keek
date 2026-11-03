use crate::checklist::{Checklist, Verdict};
use crate::timeline::{CheckId, CheckOutcome::*, Millis};

#[test]
fn every_outcome_mix_derives_its_verdict() {
    for (outcomes, verdict) in [
        (vec![Passed], Verdict::Pass),
        (vec![Passed, Passed], Verdict::Pass),
        (vec![Failed], Verdict::NeedsFixes),
        (vec![Passed, Failed], Verdict::NeedsFixes),
        (vec![Skipped], Verdict::PartlyVerified),
        (vec![Skipped, Skipped], Verdict::PartlyVerified),
        (vec![Passed, Skipped], Verdict::PartlyVerified),
        (vec![Failed, Skipped], Verdict::NeedsFixes),
        (vec![Passed, Failed, Skipped], Verdict::NeedsFixes),
    ] {
        let mut list = Checklist::new(vec!["first".into()]);
        for i in 1..outcomes.len() {
            list.add(format!("check {i}")).unwrap();
        }
        for (i, outcome) in outcomes.into_iter().enumerate() {
            list.mark(
                CheckId(i as u32 + 1),
                outcome,
                "observed".into(),
                Millis(10),
            )
            .unwrap();
        }
        assert_eq!(list.verdict().unwrap(), verdict);
    }
}

#[test]
fn marks_validate_and_replace_and_close_names_unmarked_checks() {
    let mut list = Checklist::new(vec!["button works".into()]);
    assert!(
        list.verdict()
            .unwrap_err()
            .to_string()
            .contains("1: button works")
    );
    assert!(Checklist::new(vec![]).verdict().is_err());
    assert!(list.add("  ".into()).is_err());
    assert!(
        list.mark(CheckId(2), Passed, "".into(), Millis(0))
            .unwrap_err()
            .to_string()
            .contains("1")
    );
    for outcome in [Failed, Skipped] {
        for comment in [" ", "a\nb", "a\rb"] {
            assert!(
                list.mark(CheckId(1), outcome, comment.into(), Millis(0))
                    .is_err()
            );
        }
    }
    assert!(
        list.mark(CheckId(1), Passed, "a\nb".into(), Millis(0))
            .is_err()
    );
    list.mark(CheckId(1), Failed, "broken".into(), Millis(1))
        .unwrap();
    list.mark(CheckId(1), Passed, "".into(), Millis(2)).unwrap();
    assert_eq!(list.verdict().unwrap(), Verdict::Pass);
}
#[test]
fn idle_reasons_use_singular_units_for_one_minute_and_one_second() {
    use super::IncompleteReason;
    use std::time::Duration;
    assert_eq!(
        IncompleteReason::Idle {
            limit: Duration::from_secs(60)
        }
        .sentence(),
        "keek ended the session after 1 minute without a tool call."
    );
    assert_eq!(
        IncompleteReason::Idle {
            limit: Duration::from_secs(1)
        }
        .sentence(),
        "keek ended the session after 1 second without a tool call."
    );
    assert_eq!(
        IncompleteReason::Idle {
            limit: Duration::from_secs(60)
        }
        .short(),
        "No tool call for 1 min"
    );
}
