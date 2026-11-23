use super::{Controller, EventStream, Paused, ProcessError, Span, Taking, TakingPhase};
use crate::{
    ipc::{
        ControlState, HandBackResult,
        HostShortcuts::{self, Captured, CapturedWithoutStepAway, Live},
    },
    timeline::Millis,
};
use std::time::Instant;

impl Controller {
    pub(super) async fn step_away(
        &self,
        taking: &mut Taking,
        lines: &mut EventStream,
    ) -> Result<(), ProcessError> {
        if let TakingPhase::Paused { .. } = taking.phase {
            return Ok(());
        }
        let shortcuts = taking.shortcuts;
        self.reset_submap(taking).await?;
        // Hyprland reports our own reset as a submap exit, which would trigger Done.
        *lines = None;
        let result = async {
            self.prop("no_focus", "1").await?;
            self.restore_focus(taking).await?;
            if let Err(error) = self.set_paused_border().await {
                if let Err(cleanup) = self.prop("inactive_border_color", "unset").await {
                    tracing::warn!(%cleanup, "Could not restore the border after Step away failed");
                }
                return Err(error);
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            self.restore_running_takeover(taking, lines, shortcuts)
                .await;
            self.publish_takeover(taking);
            return Err(error);
        }
        let since = self.clock.now();
        self.send_paused_notification().await;
        taking.phase = TakingPhase::Paused { since };
        self.publish_takeover(taking);
        Ok(())
    }

    async fn restore_running_takeover(
        &self,
        taking: &mut Taking,
        lines: &mut EventStream,
        shortcuts: HostShortcuts,
    ) {
        if let Err(error) = self.prop("no_focus", "0").await {
            tracing::warn!(%error, "Could not restore no_focus after Step away failed");
        }
        if let Some(window) = &self.window
            && let Err(error) = self.focus(&window.0).await
        {
            tracing::warn!(%error, "Could not focus the Session Window after Step away failed");
        }
        match shortcuts {
            Captured | CapturedWithoutStepAway => {
                let capture = async {
                    let events = self.compositor.takeover_events().await?;
                    self.submap("keek").await?;
                    Ok::<_, ProcessError>(events)
                }
                .await;
                match capture {
                    Ok(events) => {
                        *lines = events;
                        taking.shortcuts = shortcuts;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Could not recapture shortcuts after Step away failed");
                    }
                }
            }
            Live => {}
        }
    }

    pub(super) async fn done(&self, taking: &mut Taking) -> Result<Span, ProcessError> {
        let end = match taking.phase {
            TakingPhase::Running => {
                let shortcuts = taking.shortcuts;
                let reset = self.reset_submap(taking).await;
                if taking.shortcuts != shortcuts {
                    self.publish_takeover(taking);
                }
                self.prop("no_focus", "1").await?;
                self.restore_focus(taking).await?;
                reset?;
                self.clock.now()
            }
            TakingPhase::Paused { .. } => {
                let end = self.clock.now();
                self.clear_pause().await;
                end
            }
        };
        if let Err(error) = self.clipboard.clear().await {
            tracing::warn!(%error, "Could not clear the session clipboard on Done");
        }
        Ok(self.finish_takeover(taking, end).await)
    }

    pub(super) async fn finish_on_end(&self, taking: &mut Taking) {
        let end = match taking.phase {
            TakingPhase::Running => {
                if let Err(error) = self.reset_submap(taking).await {
                    tracing::warn!(%error, "Could not reset Takeover submap");
                }
                if let Err(error) = self.prop("no_focus", "1").await {
                    tracing::warn!(%error, "Could not restore no_focus");
                }
                if let Err(error) = self.restore_focus(taking).await {
                    tracing::warn!(%error, "Could not restore focus");
                }
                self.clock.now()
            }
            TakingPhase::Paused { .. } => {
                let end = self.clock.now();
                self.clear_pause().await;
                end
            }
        };
        self.finish_takeover(taking, end).await;
    }

    async fn clear_pause(&self) {
        self.dismiss_paused_notification().await;
        if let Err(error) = self.prop("inactive_border_color", "unset").await {
            tracing::warn!(%error, "Could not clear the paused border");
        }
    }

    async fn reset_submap(&self, taking: &mut Taking) -> Result<(), ProcessError> {
        match taking.shortcuts {
            Captured | CapturedWithoutStepAway => {
                self.submap("reset").await?;
                taking.shortcuts = Live;
            }
            Live => {}
        }
        Ok(())
    }

    pub(super) async fn restore_focus(&self, taking: &Taking) -> Result<(), ProcessError> {
        let clients = self.json("clients").await?;
        let candidates: Vec<_> = clients
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c.get("class").and_then(serde_json::Value::as_str) != Some("aquamarine"))
            .collect();
        fn address<'a>(c: &&'a serde_json::Value) -> Option<&'a str> {
            c.get("address").and_then(serde_json::Value::as_str)
        }
        let target = taking
            .previous
            .as_ref()
            .and_then(|previous| {
                candidates
                    .iter()
                    .find(|c| address(c) == Some(previous.0.as_str()))
                    .and_then(address)
            })
            .or_else(|| {
                candidates
                    .iter()
                    .min_by_key(|c| {
                        c.get("focusHistoryID")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(u64::MAX)
                    })
                    .and_then(address)
            })
            .filter(|address| {
                address.strip_prefix("0x").is_some_and(|hex| {
                    !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            })
            .ok_or_else(|| ProcessError::Failed {
                program: "hyprctl".into(),
                message: "Could not return focus because no other window is available. Open another window and try again.".into(),
            })?;
        self.focus(target).await?;
        let active = self.json("activewindow").await?;
        let active_address = active
            .get("address")
            .and_then(serde_json::Value::as_str)
            .filter(|address| !address.is_empty())
            .ok_or_else(|| ProcessError::Failed {
                program: "hyprctl".into(),
                message: "Could not confirm where focus moved because Hyprland returned no active window. Focus another window and try again.".into(),
            })?;
        if self
            .window
            .as_ref()
            .is_some_and(|window| active_address == window.0)
        {
            return Err(ProcessError::Failed {
                program: "hyprctl".into(),
                message: "Could not return focus because the Session Window still has it. Focus another window and try again.".into(),
            });
        }
        Ok(())
    }
    async fn finish_takeover(&self, taking: &mut Taking, end: Instant) -> Span {
        if let TakingPhase::Paused { since } = taking.phase {
            taking.paused.push(Paused { start: since, end });
        }
        let span = Span {
            start: taking.start,
            end,
            reason: taking.reason.take(),
            paused: std::mem::take(&mut taking.paused),
        };
        self.control.spans.lock().await.push(span.clone());
        self.events.send_modify(|s| {
            s.control = ControlState::HandBack {
                end_ms: self.time(end),
                result: if span.reason.is_some() {
                    HandBackResult::HelpAnswered
                } else {
                    HandBackResult::Handed
                },
                length: Millis(
                    end.saturating_duration_since(span.start)
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX),
                ),
            }
        });
        self.control.gate.open(span.clone());
        span
    }
}
