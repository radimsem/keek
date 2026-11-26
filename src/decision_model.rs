use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::config::DecisionModelConfig;
use crate::step_candidates::{Candidate, StepAction};

pub const REOBSERVE: &str = "reobserve";
pub const ABSTAIN: &str = "abstain";
const PICK: &str = "pick";
const MATCHES: &str = "matches";
const PATH: &str = "/v1/systemone";

pub type DecisionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Decision, DecisionModelError>> + Send + 'a>>;

pub trait DecisionModel: Send + Sync {
    fn decide(&self, request: DecisionRequest) -> DecisionFuture<'_>;
}

#[derive(Debug, Clone)]
pub struct DecisionRequest {
    /// Already masked: no Test Credential value reaches the model.
    pub intent: String,
    pub action: StepAction,
    pub window_title: Option<String>,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// A candidate id, `"abstain"` or `"reobserve"`.
    pub choice: String,
    pub probabilities: BTreeMap<String, f64>,
    /// The Noul probability that one candidate is the control the intent refers to.
    pub matches: f64,
    pub latency: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum DecisionModelError {
    #[error(
        "The Decision Model key is not set: environment variable {var} is missing or empty. Export {var} before starting keek, or remove api_key_env from [decision_model]."
    )]
    MissingKey { var: String },
    #[error(
        "The Decision Model did not answer within {ms} ms. Act on the window yourself, or raise timeout_ms in [decision_model]."
    )]
    Timeout { ms: u64 },
    #[error(
        "The Decision Model endpoint answered HTTP {status}. Check endpoint, model and api_key_env in [decision_model], then act on the window yourself."
    )]
    Http { status: u16 },
    #[error(
        "keek could not reach the Decision Model at {endpoint}. Check that the server is running and the endpoint is right, then act on the window yourself."
    )]
    Unreachable { endpoint: String },
    #[error(
        "The Decision Model answered in a shape keek cannot read: {reason}. Check that the endpoint speaks the Jev API, then act on the window yourself."
    )]
    Malformed { reason: String },
    #[error(
        "keek could not set up its HTTP client for the Decision Model: {reason}. Check the TLS setup of this machine and restart keek."
    )]
    Client { reason: String },
}

/// Builds the one Jev request for a step: a Choice over the candidates, and a Noul asking whether
/// any of them is the control at all.
///
/// The model sees a candidate's id, role, label and value only, never its element token.
pub fn build_request(model: &str, request: &DecisionRequest) -> Value {
    let mut state = json!({
        "intent": request.intent,
        "action": request.action,
        "candidates": request.candidates.iter().map(candidate_state).collect::<Vec<_>>(),
    });
    if let (Some(title), Some(state)) = (&request.window_title, state.as_object_mut()) {
        state.insert("window_title".to_owned(), Value::String(title.clone()));
    }

    let mut criteria = Map::new();
    for candidate in &request.candidates {
        criteria.insert(candidate.id.clone(), Value::String(describe(candidate)));
    }
    criteria.insert(
        REOBSERVE.to_owned(),
        Value::String(
            "The screen may still be changing, so the candidates may not be final.".to_owned(),
        ),
    );
    criteria.insert(
        ABSTAIN.to_owned(),
        Value::String("None of these is meant.".to_owned()),
    );

    json!({
        "model": model,
        "state": state,
        "questions": {
            PICK: {
                "type": "choice",
                "instructions": "Which control does the Verifier's intent refer to? The intent is `intent`, the action to perform on it is `action`, and the controls are `candidates`.",
                "criteria": criteria,
            },
            MATCHES: {
                "type": "noul",
                "instructions": "One of the candidates is the control the intent refers to.",
            },
        },
    })
}

fn candidate_state(candidate: &Candidate) -> Value {
    json!({
        "id": candidate.id,
        "role": candidate.role,
        "label": candidate.label,
        "value": candidate.value,
    })
}

fn describe(candidate: &Candidate) -> String {
    let mut text = format!("A {}", candidate.role);
    match &candidate.label {
        Some(label) => text.push_str(&format!(" labelled \"{label}\"")),
        None => text.push_str(" with no label"),
    }
    if let Some(value) = &candidate.value {
        text.push_str(&format!(", holding \"{value}\""));
    }
    text.push('.');
    text
}

#[derive(Deserialize)]
struct Answer {
    answers: Answers,
}

#[derive(Deserialize)]
struct Answers {
    pick: ChoiceAnswer,
    matches: NoulAnswer,
}

#[derive(Deserialize)]
struct ChoiceAnswer {
    choice: String,
    probabilities: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
struct NoulAnswer {
    noul: f64,
}

/// Reads the pick, its probabilities and the Noul probability out of a Jev answer body.
///
/// The pick is not checked against the offered ids: the caller does that before it acts.
pub fn parse_answer(
    body: &Value,
) -> Result<(String, BTreeMap<String, f64>, f64), DecisionModelError> {
    let answer = Answer::deserialize(body).map_err(|error| DecisionModelError::Malformed {
        reason: error.to_string(),
    })?;
    Ok((
        answer.answers.pick.choice,
        answer.answers.pick.probabilities,
        answer.answers.matches.noul,
    ))
}

/// A client for TypeSafe's Jev API or any server that speaks it.
pub struct JevClient {
    http: reqwest::Client,
    url: String,
    endpoint: String,
    model: String,
    timeout_ms: u64,
    key: Option<String>,
}

impl JevClient {
    /// Reads the key once from the environment variable `api_key_env` names.
    ///
    /// An empty `api_key_env` means a local server that takes no key.
    pub fn new(config: &DecisionModelConfig) -> Result<Self, DecisionModelError> {
        let key = match config.api_key_env.as_deref().filter(|var| !var.is_empty()) {
            Some(var) => Some(
                std::env::var(var)
                    .ok()
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| DecisionModelError::MissingKey {
                        var: var.to_owned(),
                    })?,
            ),
            None => None,
        };
        Self::with_key(config, key)
    }

    fn with_key(
        config: &DecisionModelConfig,
        key: Option<String>,
    ) -> Result<Self, DecisionModelError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|error| DecisionModelError::Client {
                reason: error.without_url().to_string(),
            })?;
        Ok(Self {
            http,
            url: format!("{}{PATH}", config.endpoint.trim_end_matches('/')),
            endpoint: config.endpoint.clone(),
            model: config.model.clone(),
            timeout_ms: config.timeout_ms,
            key,
        })
    }

    async fn ask(&self, request: DecisionRequest) -> Result<Decision, DecisionModelError> {
        let body = build_request(&self.model, &request);
        let mut call = self.http.post(&self.url).json(&body);
        if let Some(key) = &self.key {
            call = call.bearer_auth(key);
        }
        let started = Instant::now();
        let response = call.send().await.map_err(|error| self.transport(&error))?;
        let status = response.status();
        if !status.is_success() {
            return Err(DecisionModelError::Http {
                status: status.as_u16(),
            });
        }
        let body: Value = response.json().await.map_err(|error| {
            if error.is_timeout() {
                self.timed_out()
            } else {
                DecisionModelError::Malformed {
                    reason: "the body is not JSON".to_owned(),
                }
            }
        })?;
        let latency = started.elapsed();
        let (choice, probabilities, matches) = parse_answer(&body)?;
        Ok(Decision {
            choice,
            probabilities,
            matches,
            latency,
        })
    }

    fn transport(&self, error: &reqwest::Error) -> DecisionModelError {
        if error.is_timeout() {
            self.timed_out()
        } else {
            DecisionModelError::Unreachable {
                endpoint: self.endpoint.clone(),
            }
        }
    }

    fn timed_out(&self) -> DecisionModelError {
        DecisionModelError::Timeout {
            ms: self.timeout_ms,
        }
    }
}

impl DecisionModel for JevClient {
    fn decide(&self, request: DecisionRequest) -> DecisionFuture<'_> {
        Box::pin(self.ask(request))
    }
}

impl fmt::Debug for JevClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JevClient")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("timeout_ms", &self.timeout_ms)
            .field("key", &self.key.as_ref().map(|_| "<hidden>"))
            .finish()
    }
}

#[cfg(test)]
#[path = "decision_model_tests.rs"]
mod tests;
