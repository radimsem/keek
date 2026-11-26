use crate::decision_model::{
    Decision, DecisionFuture, DecisionModel, DecisionModelError, DecisionRequest,
};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

/// Answers each request with the next queued answer, and records every request it gets.
#[derive(Default)]
pub(super) struct FakeDecisionModel {
    answers: Mutex<VecDeque<Result<Decision, DecisionModelError>>>,
    pub(super) requests: Mutex<Vec<DecisionRequest>>,
    never: bool,
}

impl FakeDecisionModel {
    pub(super) fn answering(answers: Vec<Result<Decision, DecisionModelError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            ..Self::default()
        }
    }

    /// A model whose answer never arrives.
    pub(super) fn never() -> Self {
        Self {
            never: true,
            ..Self::default()
        }
    }
}

impl DecisionModel for FakeDecisionModel {
    fn decide(&self, request: DecisionRequest) -> DecisionFuture<'_> {
        self.requests.lock().unwrap().push(request);
        if self.never {
            return Box::pin(std::future::pending());
        }
        let answer = self.answers.lock().unwrap().pop_front();
        Box::pin(async move {
            answer.unwrap_or_else(|| {
                Err(DecisionModelError::Malformed {
                    reason: "the fake has no queued answer".into(),
                })
            })
        })
    }
}

pub(super) fn pick(choice: &str, probabilities: &[(&str, f64)], matches: f64) -> Decision {
    Decision {
        choice: choice.into(),
        probabilities: probabilities
            .iter()
            .map(|(id, p)| ((*id).to_owned(), *p))
            .collect(),
        matches,
        latency: Duration::from_millis(12),
    }
}
