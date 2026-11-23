use super::{Controller, EventStream, Paused, ProcessError, Span, Taking, TakingPhase};
use crate::{
    compositor::WindowAddress,
    ipc::HostShortcuts::{self, Captured, CapturedWithoutStepAway, Live},
};
use std::time::Instant;

enum EngageStage {
    Preparing,
    ClearingBorder,
    EnablingFocus(Instant),
    Focusing(Instant),
    EnteringSubmap(Instant),
}

impl Controller {
    pub(super) async fn eval(&self, lua: &str) -> Result<(), ProcessError> {
        let output = self.runner.output("hyprctl", &["eval", lua], None).await?;
        if output.trim() == "ok" {
            Ok(())
        } else {
            Err(ProcessError::Failed {
                program: "hyprctl eval".into(),
                message: format!(
                    "Hyprland rejected the Takeover command: {output}. Check the host configuration and retry."
                ),
            })
        }
    }
    pub(super) async fn prop(&self, prop: &str, value: &str) -> Result<(), ProcessError> {
        if let Some(window) = &self.window {
            self.eval(&format!("hl.dispatch(hl.dsp.window.set_prop({{ window = \"address:{}\", prop = \"{prop}\", value = \"{value}\" }}))", window.0)).await?;
        }
        Ok(())
    }
    pub(super) async fn focus(&self, window: &str) -> Result<(), ProcessError> {
        self.eval(&format!(
            "hl.dispatch(hl.dsp.focus({{ window = \"address:{window}\" }}))"
        ))
        .await
    }
    pub(super) async fn submap(&self, name: &str) -> Result<(), ProcessError> {
        self.eval(&format!("hl.dispatch(hl.dsp.submap(\"{name}\"))"))
            .await
    }
    pub(super) async fn json(&self, command: &str) -> Result<serde_json::Value, ProcessError> {
        let output = self
            .runner
            .output("hyprctl", &[command, "-j"], None)
            .await?;
        serde_json::from_str(&output).map_err(|e| ProcessError::Failed {
            program: "hyprctl".into(),
            message: format!("Cannot read {command}: {e}. Check Hyprland and retry."),
        })
    }
    pub(super) async fn take_control(
        &self,
        reason: Option<String>,
    ) -> Result<(Taking, EventStream), (ProcessError, Option<Taking>, Option<Span>)> {
        let permit = self.control.gate.close().await;
        let clients = match self.json("clients").await {
            Ok(clients) => clients,
            Err(error) => {
                self.control.gate.cancel();
                return Err((error, None, None));
            }
        };
        let previous = clients
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c.get("class").and_then(serde_json::Value::as_str) != Some("aquamarine"))
            .min_by_key(|c| {
                c.get("focusHistoryID")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(u64::MAX)
            })
            .and_then(|c| c.get("address"))
            .and_then(serde_json::Value::as_str)
            .filter(|address| {
                address.strip_prefix("0x").is_some_and(|hex| {
                    !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            })
            .map(|address| WindowAddress(address.to_owned()));
        let (start, shortcuts, events) = match self.engage_takeover().await {
            Ok(engaged) => engaged,
            Err((error, stage)) => {
                let start = match stage {
                    EngageStage::Preparing | EngageStage::ClearingBorder => {
                        self.control.gate.cancel();
                        return Err((error, None, None));
                    }
                    EngageStage::EnablingFocus(start)
                    | EngageStage::Focusing(start)
                    | EngageStage::EnteringSubmap(start) => start,
                };
                let mut taking = Taking {
                    start,
                    reason,
                    previous,
                    shortcuts: Live,
                    phase: TakingPhase::Running,
                    paused: Vec::new(),
                };
                return match self.done(&mut taking).await {
                    Ok(span) => Err((error, None, Some(span))),
                    Err(cleanup) => {
                        tracing::warn!(%cleanup, "Could not restore host after Takeover failed");
                        self.publish_takeover(&taking);
                        Err((error, Some(taking), None))
                    }
                };
            }
        };
        let taking = Taking {
            start,
            reason,
            previous,
            shortcuts,
            phase: TakingPhase::Running,
            paused: Vec::new(),
        };
        self.carry_host_clipboard().await;
        self.publish_takeover(&taking);
        drop(permit);
        Ok((taking, events))
    }

    pub(super) async fn resume(
        &self,
        taking: &mut Taking,
        lines: &mut EventStream,
    ) -> Result<(), ProcessError> {
        let TakingPhase::Paused { since } = taking.phase else {
            return Ok(());
        };
        let (_, shortcuts, events) = match self.engage_takeover().await {
            Ok(engaged) => engaged,
            Err((error, stage)) => {
                self.restore_paused_takeover(taking, stage).await;
                return Err(error);
            }
        };
        taking.paused.push(Paused {
            start: since,
            end: self.clock.now(),
        });
        self.dismiss_paused_notification().await;
        taking.shortcuts = shortcuts;
        taking.phase = TakingPhase::Running;
        *lines = events;
        self.carry_host_clipboard().await;
        self.publish_takeover(taking);
        Ok(())
    }

    async fn carry_host_clipboard(&self) {
        let result = match crate::takeover_clipboard::read_host_text(self.runner.as_ref()).await {
            Ok(Some(text)) => self.clipboard.copy(text).await,
            Ok(None) => return,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            tracing::warn!(%error, "Paste from the host is unavailable in this Takeover");
        }
    }

    async fn restore_paused_takeover(&self, taking: &Taking, stage: EngageStage) {
        if matches!(stage, EngageStage::Preparing) {
            return;
        }
        if !matches!(stage, EngageStage::ClearingBorder)
            && let Err(error) = self.prop("no_focus", "1").await
        {
            tracing::warn!(%error, "Could not restore no_focus after Resume failed");
        }
        if self.window.is_some()
            && matches!(
                stage,
                EngageStage::Focusing(_) | EngageStage::EnteringSubmap(_)
            )
            && let Err(error) = self.restore_focus(taking).await
        {
            tracing::warn!(%error, "Could not restore focus after Resume failed");
        }
        // The dispatch may have changed the submap before reporting a failure.
        if matches!(stage, EngageStage::EnteringSubmap(_))
            && let Err(error) = self.submap("reset").await
        {
            tracing::warn!(%error, "Could not reset the submap after Resume failed");
        }
        if let Err(error) = self.set_paused_border().await {
            tracing::warn!(%error, "Could not restore the paused border after Resume failed");
        }
    }

    async fn engage_takeover(
        &self,
    ) -> Result<(Instant, HostShortcuts, EventStream), (ProcessError, EngageStage)> {
        let preparing = |error| (error, EngageStage::Preparing);
        let binds = self.json("binds").await.map_err(preparing)?;
        let shortcuts = if has_submap(&binds) {
            if has_step_away(&binds) {
                Captured
            } else {
                CapturedWithoutStepAway
            }
        } else {
            Live
        };
        let events = match shortcuts {
            Captured | CapturedWithoutStepAway => {
                self.compositor.takeover_events().await.map_err(preparing)?
            }
            Live => None,
        };
        let start = self.clock.now();
        self.prop("inactive_border_color", "unset")
            .await
            .map_err(|error| (error, EngageStage::ClearingBorder))?;
        self.prop("no_focus", "0")
            .await
            .map_err(|error| (error, EngageStage::EnablingFocus(start)))?;
        if let Some(window) = &self.window {
            self.focus(&window.0)
                .await
                .map_err(|error| (error, EngageStage::Focusing(start)))?;
        }
        // Keep submap entry as the last fallible host step.
        match shortcuts {
            Captured | CapturedWithoutStepAway => self
                .submap("keek")
                .await
                .map_err(|error| (error, EngageStage::EnteringSubmap(start)))?,
            Live => {}
        }
        Ok((start, shortcuts, events))
    }
}

pub fn has_submap(binds: &serde_json::Value) -> bool {
    has_super_bind(binds, "Escape")
}

pub fn has_step_away(binds: &serde_json::Value) -> bool {
    has_super_bind(binds, "Tab")
}

fn has_super_bind(binds: &serde_json::Value, expected_key: &str) -> bool {
    binds.as_array().into_iter().flatten().any(|bind| {
        bind.get("submap").and_then(serde_json::Value::as_str) == Some("keek")
            && bind
                .get("key")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|key| key.eq_ignore_ascii_case(expected_key))
            && bind
                .get("modmask")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|mask| mask & 64 != 0)
    })
}
