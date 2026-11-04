use super::*;

impl Controller {
    pub(super) async fn raise_help(&self, help: &str) -> Result<(), ProcessError> {
        self.events.send_modify(|s| {
            s.control = ControlState::HelpRequest {
                reason: help.to_owned(),
                start_ms: self.time(self.clock.now()),
            }
        });
        let text = format!("{help} · {}", self.project);
        if let Err(error) = self
            .runner
            .output(
                "omarchy-notification-send",
                &[
                    "-u",
                    "normal",
                    "Agent needs you",
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
}
