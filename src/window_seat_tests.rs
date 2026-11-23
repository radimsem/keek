use super::*;
use crate::compositor::{
    CompositorFuture, EventsFuture, Placement, WindowEvents, WindowFuture, WindowPid,
    WorkspaceFuture,
};
use crate::process::ProcessError;
use std::sync::Mutex;

#[derive(Default)]
struct FakeCompositor {
    moves: Mutex<Vec<(WindowAddress, WindowSpot)>>,
    next_error: Mutex<Option<ProcessError>>,
}

impl Compositor for FakeCompositor {
    fn place_window(
        &self,
        _: Option<WindowPid>,
        _: Placement,
        _: Option<WindowEvents>,
    ) -> WindowFuture<'_> {
        Box::pin(async { panic!("WindowSeat does not place windows") })
    }

    fn workspace(&self, _: usize) -> WorkspaceFuture<'_> {
        Box::pin(async { panic!("WindowSeat does not query the workspace") })
    }

    fn move_window(&self, address: WindowAddress, spot: WindowSpot) -> CompositorFuture<'_> {
        Box::pin(async move {
            self.moves.lock().unwrap().push((address, spot));
            self.next_error.lock().unwrap().take().map_or(Ok(()), Err)
        })
    }

    fn window_events(&self) -> EventsFuture<'_> {
        Box::pin(async { panic!("WindowSeat does not watch window events") })
    }

    fn install(&self) -> CompositorFuture<'_> {
        Box::pin(async { panic!("WindowSeat does not install window rules") })
    }

    fn remove(&self) -> CompositorFuture<'_> {
        Box::pin(async { panic!("WindowSeat does not remove window rules") })
    }
}

const SHOWN: WindowSpot = WindowSpot::Shown { x: 24, y: 50 };

#[tokio::test]
async fn placement_applies_a_move_requested_before_the_window_was_placed() {
    let compositor = FakeCompositor::default();
    let seat = WindowSeat::new(SHOWN);
    let address = WindowAddress("0xa0".into());
    seat.move_to(WindowSpot::Hidden, &compositor).await;
    assert!(compositor.moves.lock().unwrap().is_empty());
    seat.placed(address.clone(), &compositor).await;
    assert_eq!(
        *compositor.moves.lock().unwrap(),
        [(address, WindowSpot::Hidden)]
    );
}

#[tokio::test]
async fn requesting_the_current_spot_does_not_move_the_window() {
    let compositor = FakeCompositor::default();
    let seat = WindowSeat::new(SHOWN);
    seat.placed(WindowAddress("0xa0".into()), &compositor).await;
    seat.move_to(SHOWN, &compositor).await;
    assert!(compositor.moves.lock().unwrap().is_empty());
}

#[tokio::test]
async fn requesting_the_same_spot_after_a_failed_move_retries_it() {
    let compositor = FakeCompositor::default();
    *compositor.next_error.lock().unwrap() = Some(ProcessError::Failed {
        program: "hyprctl".into(),
        message: "move refused".into(),
    });
    let seat = WindowSeat::new(SHOWN);
    let address = WindowAddress("0xa0".into());
    seat.placed(address.clone(), &compositor).await;
    seat.move_to(WindowSpot::Hidden, &compositor).await;
    seat.move_to(WindowSpot::Hidden, &compositor).await;
    seat.move_to(WindowSpot::Hidden, &compositor).await;
    assert_eq!(
        *compositor.moves.lock().unwrap(),
        [
            (address.clone(), WindowSpot::Hidden),
            (address, WindowSpot::Hidden)
        ]
    );
}
