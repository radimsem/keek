use super::*;

const HELP_SUMMARY: &str = "Agent needs you";

impl Controller {
    pub(super) async fn raise_help(&self, help: &str) -> Result<(), ProcessError> {
        self.events.send_modify(|s| {
            s.control = ControlState::HelpRequest {
                reason: help.to_owned(),
                start_ms: self.time(self.clock.now()),
            }
        });
        let text = format!("{help} · {}", self.project);
        // Omarchy expires a normal notification after 8 s; a critical one stays until acted on.
        if let Err(error) = self
            .runner
            .output(
                "omarchy-notification-send",
                &[
                    "-u",
                    "critical",
                    HELP_SUMMARY,
                    &text,
                    "--exec",
                    "hyprctl",
                    "eval",
                    "hl.dispatch(hl.dsp.focus({ workspace = \"name:keek\" }))",
                ],
                None,
            )
            .await
        {
            tracing::warn!(%error, "Could not send Help Request notification");
        }
        let orange = tokio::task::spawn_blocking(|| crate::viewer::theme::load().orange)
            .await
            .unwrap_or_else(|_| crate::viewer::theme::parse("").yellow);
        self.prop(
            "inactive_border_color",
            &format!("rgb({})", orange.trim_start_matches('#')),
        )
        .await
    }

    /// Takes the Help Request notification off screen once the developer has taken control. The
    /// Omarchy shell keeps a card up after its sender closes it, so keek dismisses it through the
    /// shell, which matches by summary.
    pub(super) async fn dismiss_help_notification(&self) {
        if let Err(error) = self
            .runner
            .output("omarchy-notification-dismiss", &[HELP_SUMMARY], None)
            .await
        {
            tracing::warn!(%error, "Could not dismiss the Help Request notification");
        }
    }

    /// Puts back the card state from before the Help Request and unsets the orange border.
    pub(super) async fn withdraw_help(&self, before: ControlState) -> Result<(), ProcessError> {
        self.events.send_modify(|s| s.control = before);
        self.prop("inactive_border_color", "unset").await
    }
}
