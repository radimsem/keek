use super::{Control, HelpOutcome, Instant};

impl Control {
    pub async fn mark_unanswered(&self, takeover_start: Instant) {
        for help in self.help_requests.lock().await.iter_mut() {
            if help.end == takeover_start && help.outcome == HelpOutcome::Answered {
                help.outcome = HelpOutcome::Unanswered;
                break;
            }
        }
    }
}
