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
        if let Err(error) = self.send_notification(HELP_SUMMARY, &text).await {
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
        if let Err(error) = self.dismiss_notification(HELP_SUMMARY).await {
            tracing::warn!(%error, "Could not dismiss the Help Request notification");
        }
    }

    /// Puts back the card state from before the Help Request and unsets the orange border.
    pub(super) async fn withdraw_help(&self, before: ControlState) -> Result<(), ProcessError> {
        self.events.send_modify(|s| s.control = before);
        self.prop("inactive_border_color", "unset").await
    }

    pub(super) async fn set_paused_border(&self) -> Result<(), ProcessError> {
        let yellow = tokio::task::spawn_blocking(|| crate::viewer::theme::load().yellow)
            .await
            .unwrap_or_else(|_| crate::viewer::theme::parse("").yellow);
        self.prop(
            "inactive_border_color",
            &format!("rgb({})", yellow.trim_start_matches('#')),
        )
        .await
    }

    pub(super) async fn send_paused_notification(&self) {
        if let Err(error) = self
            .send_notification(
                &self.paused_summary(),
                "The Verifier waits until you press Done. Go back to keek and click the session to carry on.",
            )
            .await
        {
            tracing::warn!(%error, "Could not send paused Takeover notification");
        }
    }

    pub(super) async fn dismiss_paused_notification(&self) {
        if let Err(error) = self.dismiss_notification(&self.paused_summary()).await {
            tracing::warn!(%error, "Could not dismiss paused Takeover notification");
        }
    }

    fn paused_summary(&self) -> String {
        format!("Takeover paused · {}", self.project)
    }

    async fn send_notification(&self, summary: &str, body: &str) -> Result<String, ProcessError> {
        let session = self
            .session
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
        let lua = format!(
            r#"hl.dispatch(hl.dsp.focus({{ workspace = "name:keek" }})); hl.dispatch(hl.dsp.event("keek-show {session}"))"#
        );
        // Omarchy expires a normal notification after 8 s; a critical one stays until acted on.
        self.runner
            .output(
                "omarchy-notification-send",
                &[
                    "-u", "critical", "-i", "keek", summary, body, "--exec", "hyprctl", "eval",
                    &lua,
                ],
                None,
            )
            .await
    }

    async fn dismiss_notification(&self, summary: &str) -> Result<String, ProcessError> {
        self.runner
            .output("omarchy-notification-dismiss", &[summary], None)
            .await
    }
}
