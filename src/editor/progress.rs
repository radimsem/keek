use super::{EditPlan, OUTPUT_FPS};
use crate::ipc::Percent;
pub(super) struct Progress {
    lengths: Vec<u64>,
    last: Option<Percent>,
}
impl Progress {
    pub(super) fn new(
        plan: &EditPlan,
        incomplete: Option<&crate::checklist::IncompleteReason>,
    ) -> Self {
        let mut lengths: Vec<_> = plan
            .segments()
            .iter()
            .map(|s| s.frames() * 1_000_000 / OUTPUT_FPS)
            .collect();
        if incomplete.is_some() {
            lengths.push(crate::title_card::TITLE_CARD.0 * 1000);
        }
        Self {
            lengths,
            last: None,
        }
    }
    pub(super) fn update(&mut self, index: usize, out_time_us: u64) -> Option<Percent> {
        let length = *self.lengths.get(index)?;
        let total: u64 = self.lengths.iter().sum();
        let completed: u64 = self.lengths.iter().take(index).sum();
        let percent = Percent(
            ((completed + out_time_us.min(length)) as u128 * 100 / u128::from(total.max(1))) as u8,
        );
        if self.last == Some(percent) {
            return None;
        }
        self.last = Some(percent);
        Some(percent)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Margins, plan_edit};
    use crate::timeline::{Millis, SessionTimeline};
    #[test]
    fn reports_whole_output_percentages_only_once() {
        let plan = plan_edit(
            &SessionTimeline::new(Millis(0)),
            &[],
            Millis(2000),
            &[],
            Margins {
                before: Millis(0),
                after: Millis(0),
            },
        );
        let mut progress = Progress::new(
            &plan,
            Some(&crate::checklist::IncompleteReason::DesktopStopped),
        );
        assert_eq!(progress.update(0, 0), Some(Percent(0)));
        assert_eq!(progress.update(0, 1), None);
        assert_eq!(progress.update(0, 1_000_000), Some(Percent(25)));
        assert_eq!(progress.update(1, 1_000_000), Some(Percent(75)));
        assert_eq!(progress.update(1, 2_100_000), Some(Percent(100)));
        assert_eq!(progress.update(1, 2_200_000), None);
    }
}
