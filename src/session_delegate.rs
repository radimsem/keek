use super::call::{CallEnd, Raced};
use super::run::LoopState;
use super::*;
use crate::catalog::CatalogError;
use crate::config::DecisionModelConfig;
use crate::credentials::{Credentials, CredentialsError};
use crate::decision_model::{DecisionModel, DecisionModelError, DecisionRequest};
use crate::step_candidates::{Candidate, CandidatesError, StepAction, candidates};
use crate::step_policy::{HandBackReason, StepDecision, decide_action};
use crate::timeline::{CandidateSummary, DelegatedOutcome, DelegatedStepEvent};
use serde::Deserialize;
use serde_json::Map;

pub const TOOL: &str = "delegate_step";
const SHOWN_CANDIDATES: usize = 5;

#[derive(Clone)]
pub struct Delegate {
    pub model: Arc<dyn DecisionModel>,
    pub config: DecisionModelConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum DelegateError {
    #[error(
        "delegate_step is off because keek has no Decision Model. The developer must add a [decision_model] table to keek's config to turn it on. Until then, act on the window yourself."
    )]
    Off,
    #[error(
        "delegate_step arguments are not valid: {0}. Pass intent, action and window, plus text for type or option for select."
    )]
    Arguments(serde_json::Error),
    #[error(
        "delegate_step with action {action} needs {field}. Pass {field}, or act on the window yourself."
    )]
    Missing {
        action: &'static str,
        field: &'static str,
    },
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Candidates(#[from] CandidatesError),
    #[error(transparent)]
    Driver(#[from] DriverError),
}

#[derive(Deserialize)]
struct StepArgs {
    intent: String,
    action: StepAction,
    text: Option<String>,
    option: Option<String>,
    window: Map<String, Value>,
}

/// A step that passed every check made before the window is read.
struct Planned {
    /// Masked, with `${NAME}` left as written.
    intent: String,
    action: StepAction,
    observe: Value,
    tool: &'static str,
    /// Substituted, and still without the element token.
    act: Value,
}

struct Done {
    reply: Result<Value, Response>,
    acted: bool,
    is_error: bool,
    event: Option<DelegatedStepEvent>,
}
impl Done {
    fn failed(error: DelegateError) -> Self {
        Self::stopped(refusal(error))
    }

    fn stopped(response: Response) -> Self {
        Self {
            reply: Err(response),
            acted: false,
            is_error: true,
            event: None,
        }
    }
}

impl Actor {
    pub(super) async fn delegate_step(
        &mut self,
        state: &mut LoopState,
        inbox: &mut Inbox,
        arguments: Value,
    ) -> (Response, CallEnd) {
        let Some(delegate) = self.delegate.clone() else {
            return (refusal(DelegateError::Off), CallEnd::Replied);
        };
        let planned = match self.plan_step(arguments) {
            Ok(planned) => planned,
            Err(error) => return (refusal(error), CallEnd::Replied),
        };
        let start = self.clock.now();
        let action_ms = self.control_time(start);
        let start_ms = timeline_time(state.zero, start);
        // The step runs while the actor still answers a close, so it cannot borrow the actor.
        let credentials = self.credentials.clone();
        let raced = self
            .race(state, inbox, |driver| {
                run_step(&delegate, &credentials, driver, planned, start_ms)
            })
            .await;
        let (done, end) = match raced {
            Raced::Finished(done) => (done, CallEnd::Replied),
            Raced::Ended(reason) => (
                Done::stopped(refusal(reason.refusal())),
                CallEnd::End(reason),
            ),
            Raced::Closing(close, ready) => (
                Done::stopped(self.closing_refusal(TOOL)),
                CallEnd::Close(close, ready),
            ),
        };
        let end_ms = timeline_time(state.zero, self.clock.now());
        state.timeline.push(TimelineEvent::Call {
            tool: TOOL.into(),
            action: done.acted,
            start_ms,
            end_ms,
            is_error: done.is_error,
        });
        if let Some(event) = done.event {
            state.timeline.push(TimelineEvent::DelegatedStep(event));
        }
        if done.acted {
            state.last_action = Some(LastAction {
                tool: TOOL.into(),
                time_ms: action_ms,
            });
            self.publish(SessionState::Open, &state.last_action);
        }
        state.deadline = tokio::time::Instant::now() + self.idle_limit;
        let response = match done.reply {
            Ok(mut value) => {
                if self.reminder.record_call(&self.checklist, &mut value) {
                    state
                        .timeline
                        .push(TimelineEvent::Reminder { at_ms: end_ms });
                    self.events.send_modify(|snapshot| snapshot.reminded = true);
                }
                Response::ToolResult(value)
            }
            Err(response) => response,
        };
        self.persist(&state.timeline).await;
        (response, end)
    }

    fn plan_step(&self, arguments: Value) -> Result<Planned, DelegateError> {
        let args: StepArgs = serde_json::from_value(arguments).map_err(DelegateError::Arguments)?;
        let missing = |action, field| DelegateError::Missing { action, field };
        let (tool, input) = match args.action {
            StepAction::Click => ("click", None),
            StepAction::DoubleClick => ("double_click", None),
            StepAction::RightClick => ("right_click", None),
            StepAction::Type => (
                "type_text",
                Some(("text", args.text.ok_or(missing("type", "text"))?)),
            ),
            // On a combo box or list, Cua's set_value picks the option with this name.
            StepAction::Select => (
                "set_value",
                Some(("value", args.option.ok_or(missing("select", "option"))?)),
            ),
        };
        let mut act = Map::new();
        // Cua reads the window from the element token, but click and set_value still require pid.
        if let Some(pid) = args.window.get("pid") {
            act.insert("pid".into(), pid.clone());
        }
        if let Some((key, input)) = input {
            act.insert(key.into(), input.into());
        }
        // The intent never reaches Cua; substituting a copy refuses an unknown name before the model is asked.
        self.credentials
            .substitute(&mut Value::String(args.intent.clone()))?;
        let (mut act, _) = self.catalog.prepare_call(tool, &self.name, act.into())?;
        self.credentials.substitute(&mut act)?;
        let (observe, _) =
            self.catalog
                .prepare_call("get_window_state", &self.name, args.window.into())?;
        Ok(Planned {
            intent: self.credentials.mask(&args.intent),
            action: args.action,
            observe,
            tool,
            act,
        })
    }
}

async fn run_step(
    delegate: &Delegate,
    credentials: &Credentials,
    driver: &mut dyn Driver,
    planned: Planned,
    at_ms: Millis,
) -> Done {
    let observed =
        match call_driver(driver, "get_window_state", planned.observe, DRIVER_LIMIT).await {
            Ok(value) if value["isError"] == true => {
                return Done {
                    reply: Ok(refusal_hint::add(driver, value).await),
                    acted: false,
                    is_error: true,
                    event: None,
                };
            }
            Ok(value) => credentials.mask_value(value),
            Err(error) => return Done::failed(error.into()),
        };
    let found = match candidates(&observed, planned.action, &planned.intent) {
        Ok(found) => found,
        Err(error) => return Done::failed(error.into()),
    };
    let window_title = observed
        .get("structuredContent")
        .unwrap_or(&observed)
        .get("window_title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let request = DecisionRequest {
        intent: planned.intent.clone(),
        action: planned.action,
        window_title,
        candidates: found.clone(),
    };
    let config = &delegate.config;
    let asked = tokio::time::Instant::now();
    let answer = tokio::time::timeout(
        Duration::from_millis(config.timeout_ms),
        delegate.model.decide(request),
    )
    .await
    .unwrap_or(Err(DecisionModelError::Timeout {
        ms: config.timeout_ms,
    }));
    let (decision, step, latency) = match answer {
        Ok(decision) => {
            let step = decide_action(&decision, &found, config.threshold);
            let latency = decision.latency;
            (Some(decision), step, latency)
        }
        Err(error) => (
            None,
            StepDecision::HandBack(HandBackReason::ModelFailed(error.to_string())),
            asked.elapsed(),
        ),
    };
    let latency_ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX);
    let probabilities = decision
        .as_ref()
        .map(|decision| decision.probabilities.clone())
        .unwrap_or_default();
    let mut event = DelegatedStepEvent {
        at_ms,
        intent: planned.intent,
        action: planned.action,
        candidates: found.iter().map(summary).collect(),
        probabilities,
        pick: decision.map(|decision| decision.choice),
        outcome: DelegatedOutcome::Acted,
        threshold: config.threshold,
        model: config.model.clone(),
        latency_ms,
    };
    let (candidate, probability) = match step {
        StepDecision::Act {
            candidate,
            probability,
        } => (candidate, probability),
        StepDecision::HandBack(reason) => {
            let text = hand_back_text(&reason, config.threshold, &found, &event.probabilities);
            let shown = ranked(&found, &event.probabilities);
            let value = json!({
                "content": [{"type": "text", "text": text}],
                "structuredContent": {"delegated_step": {
                    "acted": false,
                    "reason": reason,
                    "candidates": shown.iter().map(|(candidate, probability)| json!({
                        "id": candidate.id,
                        "role": candidate.role,
                        "label": candidate.label,
                        "probability": probability,
                    })).collect::<Vec<_>>(),
                    "decision_ms": latency_ms,
                }},
            });
            event.outcome = DelegatedOutcome::HandedBack(reason);
            return Done {
                reply: Ok(value),
                acted: false,
                is_error: false,
                event: Some(event),
            };
        }
    };
    let mut act = planned.act;
    if let Some(arguments) = act.as_object_mut() {
        arguments.insert(
            "element_token".into(),
            candidate.element_token.as_str().into(),
        );
    }
    let (reply, is_error) = match call_driver(driver, planned.tool, act, DRIVER_LIMIT).await {
        Ok(value) => {
            let mut value = refusal_hint::add(driver, value).await;
            let is_error = value["isError"] == true;
            add_pick(&mut value, &candidate, probability, latency_ms);
            (Ok(value), is_error)
        }
        Err(error) => (Err(refusal(DelegateError::from(error))), true),
    };
    Done {
        reply,
        acted: true,
        is_error,
        event: Some(event),
    }
}

fn summary(candidate: &Candidate) -> CandidateSummary {
    CandidateSummary {
        id: candidate.id.clone(),
        role: candidate.role.clone(),
        label: candidate.label.clone(),
    }
}

fn describe(candidate: &Candidate, probability: Option<f64>) -> String {
    let mut text = format!("{} {}", candidate.id, candidate.role);
    if let Some(label) = &candidate.label {
        text.push_str(&format!(" {label:?}"));
    }
    if let Some(probability) = probability {
        text.push_str(&format!(" {probability:.2}"));
    }
    text
}

fn add_pick(value: &mut Value, candidate: &Candidate, probability: f64, latency_ms: u64) {
    let Some(reply) = value.as_object_mut() else {
        return;
    };
    let line = format!(
        "keek acted on {} after a {latency_ms} ms decision.",
        describe(candidate, Some(probability))
    );
    if let Some(content) = reply.get_mut("content").and_then(Value::as_array_mut) {
        content.push(json!({"type": "text", "text": line}));
    }
    let pick = json!({"pick": candidate.id, "probability": probability, "decision_ms": latency_ms});
    match reply
        .get_mut("structuredContent")
        .and_then(Value::as_object_mut)
    {
        Some(structured) => {
            structured.insert("delegated_step".into(), pick);
        }
        None => {
            reply.insert("structuredContent".into(), json!({"delegated_step": pick}));
        }
    }
}

/// The candidates with their probabilities, highest first and at most `SHOWN_CANDIDATES`.
fn ranked<'a>(
    found: &'a [Candidate],
    probabilities: &std::collections::BTreeMap<String, f64>,
) -> Vec<(&'a Candidate, Option<f64>)> {
    let mut ranked: Vec<_> = found
        .iter()
        .map(|candidate| (candidate, probabilities.get(&candidate.id).copied()))
        .collect();
    ranked.sort_by(|a, b| b.1.unwrap_or(0.0).total_cmp(&a.1.unwrap_or(0.0)));
    ranked.truncate(SHOWN_CANDIDATES);
    ranked
}

fn hand_back_text(
    reason: &HandBackReason,
    threshold: f64,
    found: &[Candidate],
    probabilities: &std::collections::BTreeMap<String, f64>,
) -> String {
    let shown = ranked(found, probabilities)
        .into_iter()
        .map(|(candidate, probability)| describe(candidate, probability))
        .collect::<Vec<_>>();
    let shown = if shown.is_empty() {
        "none".to_owned()
    } else {
        shown.join(", ")
    };
    format!(
        "No action taken: {}. Act on the window yourself. Top candidates: {shown}.",
        reason_text(reason, threshold)
    )
}

fn reason_text(reason: &HandBackReason, threshold: f64) -> String {
    match reason {
        HandBackReason::Abstained => "the Decision Model abstained".into(),
        HandBackReason::Reobserve => {
            "the Decision Model asked to look again, because the window may still be changing"
                .into()
        }
        HandBackReason::LowConfidence { probability } => format!(
            "the best pick had probability {probability:.2}, below the threshold of {threshold:.2}"
        ),
        HandBackReason::NoMatch { matches } => format!(
            "the Decision Model gave {matches:.2} that any candidate matches the intent, below the threshold of {threshold:.2}"
        ),
        HandBackReason::NotOffered { choice } => {
            format!("the Decision Model picked {choice:?}, which is not an offered candidate")
        }
        HandBackReason::ModelFailed(message) => {
            let first = message.split(". ").next().unwrap_or(message);
            let mut chars = first.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_lowercase().chain(chars).collect::<String>()
            })
        }
    }
}
