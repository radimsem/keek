use std::{sync::Arc, time::Instant};
use tokio::sync::{Mutex, OwnedMutexGuard, watch};
#[derive(Debug, Clone)]
pub struct Paused {
    pub start: Instant,
    pub end: Instant,
}
#[derive(Debug, Clone)]
pub struct Span {
    pub start: Instant,
    pub end: Instant,
    pub reason: Option<String>,
    pub paused: Vec<Paused>,
}
#[derive(Clone)]
enum State {
    Verifier,
    Developer(Generation, Box<State>),
    HandBack(Generation, Span),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Generation(u64);
#[derive(Debug, Clone, Copy)]
pub enum Arrival {
    Ready(Generation),
    DuringTakeover(Generation),
}
impl Default for Arrival {
    fn default() -> Self {
        Self::Ready(Generation::default())
    }
}
pub enum Entry {
    Run(OwnedMutexGuard<()>),
    HandBack(Span),
}
#[derive(Clone)]
pub struct Gate {
    permit: Arc<Mutex<()>>,
    state: watch::Sender<State>,
    history: watch::Sender<Vec<(Generation, Span)>>,
}
impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}
impl Gate {
    pub fn new() -> Self {
        Self {
            permit: Arc::new(Mutex::new(())),
            state: watch::channel(State::Verifier).0,
            history: watch::channel(Vec::new()).0,
        }
    }
    pub fn arrival(&self) -> Arrival {
        match &*self.state.borrow() {
            State::Verifier => Arrival::Ready(Generation::default()),
            State::Developer(generation, _) => Arrival::DuringTakeover(*generation),
            State::HandBack(generation, _) => Arrival::Ready(*generation),
        }
    }
    pub fn handed_back(&self, arrival: Arrival) -> Option<Span> {
        self.history.borrow().iter().find_map(|(generation, span)| {
            let matches = match arrival {
                Arrival::Ready(at) => generation.0 > at.0,
                Arrival::DuringTakeover(at) => *generation == at,
            };
            matches.then(|| span.clone())
        })
    }
    pub async fn enter(&self) -> Entry {
        self.enter_for(self.arrival()).await
    }
    pub async fn enter_for(&self, arrival: Arrival) -> Entry {
        let mut state = self.state.subscribe();
        loop {
            let permit = self.permit.clone().lock_owned().await;
            let current = state.borrow_and_update().clone();
            match current {
                State::Verifier => return Entry::Run(permit),
                State::HandBack(generation, span) => {
                    let held = match arrival {
                        Arrival::Ready(at) => generation.0 > at.0,
                        Arrival::DuringTakeover(at) => generation.0 >= at.0,
                    };
                    return if held {
                        Entry::HandBack(span)
                    } else {
                        Entry::Run(permit)
                    };
                }
                State::Developer(_, _) => {
                    drop(permit);
                }
            }
            let _ = state.changed().await;
        }
    }
    pub async fn close(&self) -> OwnedMutexGuard<()> {
        self.state.send_modify(|state| {
            let generation = match state {
                State::Verifier => Generation(1),
                State::Developer(generation, _) | State::HandBack(generation, _) => {
                    Generation(generation.0.saturating_add(1))
                }
            };
            *state = State::Developer(generation, Box::new(state.clone()));
        });
        self.permit.clone().lock_owned().await
    }
    pub fn cancel(&self) {
        self.state.send_modify(|state| {
            if let State::Developer(_, previous) = state {
                *state = (**previous).clone();
            }
        });
    }
    pub fn open(&self, span: Span) {
        self.state.send_modify(|state| {
            let generation = match state {
                State::Verifier => Generation::default(),
                State::Developer(generation, _) | State::HandBack(generation, _) => *generation,
            };
            self.history
                .send_modify(|history| history.push((generation, span.clone())));
            *state = State::HandBack(generation, span.clone());
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn a_call_waits_for_done_and_receives_the_hand_back() {
        let gate = Gate::new();
        let now = std::time::Instant::now();
        let permit = gate.close().await;
        drop(permit);
        let held = tokio::spawn({
            let gate = gate.clone();
            async move { gate.enter().await }
        });
        tokio::task::yield_now().await;
        assert!(!held.is_finished());
        gate.open(Span {
            start: now,
            end: now + std::time::Duration::from_secs(42),
            reason: Some("2FA code".into()),
            paused: vec![],
        });
        assert!(matches!(held.await.unwrap(), Entry::HandBack(_)));
    }
    #[tokio::test(start_paused = true)]
    async fn take_control_waits_for_the_outstanding_permit() {
        let gate = Gate::new();
        let Entry::Run(permit) = gate.enter().await else {
            panic!("unexpected hand-back")
        };
        let taking = tokio::spawn({
            let gate = gate.clone();
            async move { gate.close().await }
        });
        tokio::task::yield_now().await;
        assert!(!taking.is_finished());
        drop(permit);
        drop(taking.await.unwrap());
    }
}
