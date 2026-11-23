use super::gate::{Gate, Span};
use crate::{
    clock::Clock,
    compositor::{Compositor, TakeoverEvents, WindowAddress},
    ipc::{ControlState, HandBackResult, HostShortcuts, SessionSnapshot},
    process::{ProcessError, ProcessRunner},
    timeline::{HelpOutcome, Millis},
};
use std::{sync::Arc, time::Instant};
use tokio::sync::{Mutex, mpsc, oneshot, watch};

pub enum Command {
    TakeControl,
    Done,
    Help {
        arrival: super::gate::Arrival,
        reason: String,
        reply: oneshot::Sender<Span>,
    },
    /// Sent after a Help Request's waiter went away. The controller answers once the card has left Help Request.
    Withdraw(oneshot::Sender<()>),
    Stop(oneshot::Sender<()>),
}
/// A Help Request from when it was raised until Take control or withdrawal.
#[derive(Debug, Clone)]
pub struct HelpSpan {
    pub reason: String,
    pub start: Instant,
    pub end: Instant,
    pub outcome: HelpOutcome,
}
#[derive(Clone)]
pub struct Control {
    pub gate: Gate,
    pub commands: mpsc::Sender<Command>,
    spans: Arc<Mutex<Vec<Span>>>,
    help_requests: Arc<Mutex<Vec<HelpSpan>>>,
}
impl Control {
    pub fn new() -> (Self, mpsc::Receiver<Command>) {
        let (commands, receiver) = mpsc::channel(16);
        (
            Self {
                gate: Gate::new(),
                commands,
                spans: Arc::new(Mutex::new(Vec::new())),
                help_requests: Arc::new(Mutex::new(Vec::new())),
            },
            receiver,
        )
    }
    pub async fn drain(&self) -> Vec<Span> {
        std::mem::take(&mut *self.spans.lock().await)
    }
    pub async fn drain_help_requests(&self) -> Vec<HelpSpan> {
        std::mem::take(&mut *self.help_requests.lock().await)
    }
    pub async fn stop(&self) {
        let (tx, rx) = oneshot::channel();
        if self.commands.send(Command::Stop(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
}
pub struct Controller {
    pub control: Control,
    pub runner: Arc<dyn ProcessRunner>,
    pub compositor: Arc<dyn Compositor>,
    pub clock: Arc<dyn Clock + Send + Sync>,
    pub events: watch::Sender<SessionSnapshot>,
    pub opened: Instant,
    pub window: Option<WindowAddress>,
    pub project: String,
}
struct Taking {
    start: Instant,
    reason: Option<String>,
    previous: Option<WindowAddress>,
    shortcuts: HostShortcuts,
}
impl Controller {
    fn time(&self, now: Instant) -> Millis {
        Millis(
            now.saturating_duration_since(self.opened)
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        )
    }
    async fn eval(&self, lua: &str) -> Result<(), ProcessError> {
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
    async fn prop(&self, prop: &str, value: &str) -> Result<(), ProcessError> {
        if let Some(window) = &self.window {
            self.eval(&format!("hl.dispatch(hl.dsp.window.set_prop({{ window = \"address:{}\", prop = \"{prop}\", value = \"{value}\" }}))", window.0)).await?;
        }
        Ok(())
    }
    async fn focus(&self, window: &str) -> Result<(), ProcessError> {
        self.eval(&format!(
            "hl.dispatch(hl.dsp.focus({{ window = \"address:{window}\" }}))"
        ))
        .await
    }
    async fn json(&self, command: &str) -> Result<serde_json::Value, ProcessError> {
        let output = self
            .runner
            .output("hyprctl", &[command, "-j"], None)
            .await?;
        serde_json::from_str(&output).map_err(|e| ProcessError::Failed {
            program: "hyprctl".into(),
            message: format!("Cannot read {command}: {e}. Check Hyprland and retry."),
        })
    }
    async fn take(
        &self,
        reason: Option<String>,
    ) -> Result<
        (Taking, Option<Box<dyn TakeoverEvents>>),
        (ProcessError, Option<Taking>, Option<Span>),
    > {
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
        let binds = match self.json("binds").await {
            Ok(binds) => binds,
            Err(error) => {
                self.control.gate.cancel();
                return Err((error, None, None));
            }
        };
        let shortcuts = if has_submap(&binds) {
            HostShortcuts::Captured
        } else {
            HostShortcuts::Live
        };
        let events = match shortcuts {
            HostShortcuts::Captured => match self.compositor.takeover_events().await {
                Ok(events) => events,
                Err(error) => {
                    self.control.gate.cancel();
                    return Err((error, None, None));
                }
            },
            HostShortcuts::Live => None,
        };
        let start = self.clock.now();
        let mut taking = Taking {
            start,
            previous,
            reason,
            shortcuts: HostShortcuts::Live,
        };
        if let Err(error) = self.prop("inactive_border_color", "unset").await {
            self.control.gate.cancel();
            return Err((error, None, None));
        }
        let result = async {
            self.prop("no_focus", "0").await?;
            if let Some(window) = &self.window {
                self.focus(&window.0).await?;
            }
            match shortcuts {
                HostShortcuts::Captured => {
                    self.eval("hl.dispatch(hl.dsp.submap(\"keek\"))").await?;
                    taking.shortcuts = HostShortcuts::Captured;
                }
                HostShortcuts::Live => {}
            }
            Ok::<(), ProcessError>(())
        }
        .await;
        if let Err(error) = result {
            return match self.done(&mut taking).await {
                Ok(span) => Err((error, None, Some(span))),
                Err(cleanup) => {
                    tracing::warn!(%cleanup, "Could not restore host after Takeover failed");
                    self.events.send_modify(|s| {
                        s.control = ControlState::Takeover {
                            start_ms: self.time(start),
                            shortcuts: taking.shortcuts,
                        }
                    });
                    Err((error, Some(taking), None))
                }
            };
        }
        self.events.send_modify(|s| {
            s.control = ControlState::Takeover {
                start_ms: self.time(start),
                shortcuts,
            }
        });
        drop(permit);
        Ok((taking, events))
    }
    async fn done(&self, taking: &mut Taking) -> Result<Span, ProcessError> {
        let reset = if taking.shortcuts == HostShortcuts::Captured {
            let result = self.eval("hl.dispatch(hl.dsp.submap(\"reset\"))").await;
            if result.is_ok() {
                taking.shortcuts = HostShortcuts::Live;
                self.events.send_modify(|s| {
                    s.control = ControlState::Takeover {
                        start_ms: self.time(taking.start),
                        shortcuts: HostShortcuts::Live,
                    };
                });
            }
            result
        } else {
            Ok(())
        };
        self.prop("no_focus", "1").await?;
        self.restore_focus(taking).await?;
        reset?;
        Ok(self.finish_takeover(taking).await)
    }
    async fn restore_focus(&self, taking: &Taking) -> Result<(), ProcessError> {
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
                message: "No other window can receive focus. Open a window and press Done again."
                    .into(),
            })?;
        self.focus(target).await?;
        let active = self.json("activewindow").await?;
        let active_address = active
            .get("address")
            .and_then(serde_json::Value::as_str)
            .filter(|address| !address.is_empty())
            .ok_or_else(|| ProcessError::Failed {
                program: "hyprctl".into(),
                message: "Cannot confirm the active window. Press Done again.".into(),
            })?;
        if self
            .window
            .as_ref()
            .is_some_and(|window| active_address == window.0)
        {
            return Err(ProcessError::Failed {
                program: "hyprctl".into(),
                message: "The Session Window still has focus. Press Done again.".into(),
            });
        }
        Ok(())
    }
    async fn finish_takeover(&self, taking: &Taking) -> Span {
        let end = self.clock.now();
        let span = Span {
            start: taking.start,
            end,
            reason: taking.reason.clone(),
        };
        self.control.spans.lock().await.push(span.clone());
        self.events.send_modify(|s| {
            s.control = ControlState::HandBack {
                end_ms: self.time(end),
                result: if taking.reason.is_some() {
                    HandBackResult::HelpAnswered
                } else {
                    HandBackResult::Handed
                },
                length: Millis(
                    end.saturating_duration_since(taking.start)
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX),
                ),
            }
        });
        self.control.gate.open(span.clone());
        span
    }
    /// Records the open Help Request, if any, as ending at `end`.
    async fn close_help(
        &self,
        open: &mut Option<(String, Instant)>,
        end: Instant,
        outcome: HelpOutcome,
    ) {
        if let Some((reason, start)) = open.take() {
            // A withdrawn Help Request leaves its notification up (docs/viewer.md).
            if outcome == HelpOutcome::Answered {
                self.dismiss_help_notification().await;
            }
            self.control.help_requests.lock().await.push(HelpSpan {
                reason,
                start,
                end,
                outcome,
            });
        }
    }
    async fn finish_on_end(&self, taking: &mut Taking) {
        if taking.shortcuts == HostShortcuts::Captured
            && let Err(error) = self.eval("hl.dispatch(hl.dsp.submap(\"reset\"))").await
        {
            tracing::warn!(%error, "Could not reset Takeover submap");
        }
        if let Err(error) = self.prop("no_focus", "1").await {
            tracing::warn!(%error, "Could not restore no_focus");
        }
        if let Err(error) = self.restore_focus(taking).await {
            tracing::warn!(%error, "Could not restore focus");
        }
        self.finish_takeover(taking).await;
    }
    pub async fn run(
        self,
        mut commands: mpsc::Receiver<Command>,
        mut ended: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    ) {
        let mut taking: Option<Taking> = None;
        let mut lines: Option<Box<dyn TakeoverEvents>> = None;
        let mut help: Option<(String, Instant)> = None;
        let mut before_help = ControlState::default();
        let mut waiting: Vec<oneshot::Sender<Span>> = Vec::new();
        loop {
            let command = tokio::select! {
                biased;
                () = &mut ended => None,
                command = commands.recv() => command,
                event = async { match lines.as_mut() { Some(lines) => lines.next().await, None => std::future::pending().await } } => {
                    match event {
                        Ok(Some(event)) => match event {
                            crate::hypr::HyprEvent::Submap(name) if name != "keek" => Some(Command::Done),
                            crate::hypr::HyprEvent::Submap(_) | crate::hypr::HyprEvent::Workspace(_) | crate::hypr::HyprEvent::FocusedMonitor(_) | crate::hypr::HyprEvent::MoveWorkspace{..} | crate::hypr::HyprEvent::OpenWindow{..} | crate::hypr::HyprEvent::CloseWindow(_) => continue,
                        },
                        Ok(None) => { tracing::warn!("Hyprland event stream ended"); lines = None; continue; }
                        Err(error) => { tracing::warn!(%error, "Hyprland event stream failed"); lines = None; continue; }
                    }
                }
            };
            let result = match command {
                Some(Command::TakeControl) if taking.is_none() => {
                    let reason = help.as_ref().map(|(reason, _)| reason.clone());
                    match self.take(reason).await {
                        Ok((active, events)) => {
                            self.close_help(&mut help, active.start, HelpOutcome::Answered)
                                .await;
                            taking = Some(active);
                            lines = events;
                            Ok(())
                        }
                        Err((error, active, span)) => {
                            let started = active
                                .as_ref()
                                .map(|active| active.start)
                                .or(span.as_ref().map(|span| span.start));
                            if let Some(started) = started {
                                self.close_help(&mut help, started, HelpOutcome::Answered)
                                    .await;
                            }
                            taking = active;
                            if let Some(span) = span {
                                for reply in waiting.drain(..) {
                                    let _ = reply.send(span.clone());
                                }
                            }
                            Err(error)
                        }
                    }
                }
                Some(Command::TakeControl) => Ok(()),
                Some(Command::Done) => {
                    lines = None;
                    if let Some(active) = taking.as_mut() {
                        match self.done(active).await {
                            Ok(span) => {
                                taking = None;
                                for reply in waiting.drain(..) {
                                    let _ = reply.send(span.clone());
                                }
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    } else {
                        Ok(())
                    }
                }
                Some(Command::Help {
                    arrival,
                    reason,
                    reply,
                }) => {
                    if let Some(span) = self.control.gate.handed_back(arrival) {
                        let _ = reply.send(span);
                        continue;
                    }
                    waiting.push(reply);
                    if taking.is_none() {
                        help = Some((reason.clone(), self.clock.now()));
                        before_help = self.events.borrow().control.clone();
                        self.raise_help(&reason).await
                    } else {
                        Ok(())
                    }
                }
                Some(Command::Withdraw(acknowledged)) => {
                    waiting.retain(|waiter| !waiter.is_closed());
                    let result = if taking.is_none() && waiting.is_empty() && help.is_some() {
                        self.close_help(&mut help, self.clock.now(), HelpOutcome::Withdrawn)
                            .await;
                        self.withdraw_help(before_help.clone()).await
                    } else {
                        Ok(())
                    };
                    let _ = acknowledged.send(());
                    result
                }
                Some(Command::Stop(reply)) => {
                    if let Some(mut active) = taking.take() {
                        self.finish_on_end(&mut active).await;
                    }
                    self.close_help(&mut help, self.clock.now(), HelpOutcome::Withdrawn)
                        .await;
                    let _ = reply.send(());
                    break;
                }
                None => {
                    if let Some(mut active) = taking.take() {
                        self.finish_on_end(&mut active).await;
                    }
                    self.close_help(&mut help, self.clock.now(), HelpOutcome::Withdrawn)
                        .await;
                    break;
                }
            };
            if let Err(error) = result {
                tracing::warn!(%error,"Takeover host step failed");
            }
        }
    }
}
#[path = "session_control_help.rs"]
mod help;
#[path = "session_control_unanswered.rs"]
mod unanswered;

pub fn has_submap(binds: &serde_json::Value) -> bool {
    binds.as_array().into_iter().flatten().any(|b| {
        b.get("submap").and_then(serde_json::Value::as_str) == Some("keek")
            && b.get("key")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|key| key.eq_ignore_ascii_case("Escape"))
            && b.get("modmask")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|mask| mask & 64 != 0)
    })
}

#[cfg(test)]
#[path = "session_control_tests.rs"]
mod tests;
