use crate::compositor::{Compositor, WindowAddress, WindowSpot};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct WindowSeat(Arc<Seat>);

struct Seat {
    address: OnceLock<WindowAddress>,
    spots: Mutex<Spots>,
}

#[cfg(test)]
#[path = "window_seat_tests.rs"]
mod tests;

struct Spots {
    current: WindowSpot,
    wanted: WindowSpot,
}

impl WindowSeat {
    pub fn new(initial: WindowSpot) -> Self {
        Self(Arc::new(Seat {
            address: OnceLock::new(),
            spots: Mutex::new(Spots {
                current: initial,
                wanted: initial,
            }),
        }))
    }

    pub async fn placed(&self, address: WindowAddress, compositor: &dyn Compositor) {
        let mut spots = self.0.spots.lock().await;
        let _ = self.0.address.set(address);
        self.apply(&mut spots, compositor).await;
    }

    pub async fn move_to(&self, spot: WindowSpot, compositor: &dyn Compositor) {
        let mut spots = self.0.spots.lock().await;
        spots.wanted = spot;
        self.apply(&mut spots, compositor).await;
    }

    pub fn address(&self) -> Option<WindowAddress> {
        self.0.address.get().cloned()
    }

    async fn apply(&self, spots: &mut Spots, compositor: &dyn Compositor) {
        if spots.current != spots.wanted
            && let Some(address) = self.address()
        {
            match compositor.move_window(address, spots.wanted).await {
                Ok(()) => spots.current = spots.wanted,
                Err(error) => tracing::warn!(%error, "could not move the Session Window"),
            }
        }
    }
}
