use std::time::Instant;

pub trait Clock {
    fn now(&self) -> Instant;
    fn local_time(&self) -> chrono::DateTime<chrono::Local> {
        chrono::Local::now()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
