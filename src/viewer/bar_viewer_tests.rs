use super::*;
use crate::{
    checklist::{SessionEnd, Verdict},
    ipc::{ControlState, EndedSession, SessionState},
    revision::Revision,
    slots::Slot,
    timeline::Millis,
    viewer::{NoopViewer, Still, ViewerCommand},
};
use std::{path::Path, sync::Mutex, time::Duration};

const RAIL: Slot = Slot {
    x: 2856,
    y: 50,
    width: 560,
    height: 384,
};

fn info(slot: usize, spot: CardSpot) -> SessionInfo {
    SessionInfo {
        name: "billing-12-00-00".parse().unwrap(),
        project: "billing".into(),
        branch: Some("fix/17-invite-role".into()),
        goal: "An invited member gets the Editor role".into(),
        opened: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        slot: SlotId(slot),
        placement: focus_geometry(),
        monitor: "DP-1".into(),
        spot,
    }
}

fn focus_geometry() -> crate::slots::CardGeometry {
    crate::slots::geometry(
        Slot {
            x: 24,
            y: 50,
            width: 2822,
            height: 1298,
        },
        None,
    )
}

fn working() -> SessionSnapshot {
    SessionSnapshot {
        control: ControlState::Verifier,
        state: SessionState::Open,
        checks: Vec::new(),
        last_action: None,
        reminded: false,
    }
}

fn ended() -> SessionSnapshot {
    SessionSnapshot {
        state: SessionState::Ended(Box::new(EndedSession {
            end: SessionEnd::Verdict(Verdict::Pass),
            revision: Revision {
                branch: None,
                commit: "e4f09b234".into(),
                dirty: false,
            },
            attempt: None,
            verification: None,
            folder: None,
            video_length: Millis(0),
            actions: 0,
            frame: None,
            elapsed: Millis(60_000),
        })),
        ..working()
    }
}

fn read(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

async fn until_written(
    path: &Path,
    done: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(state) = read(path).filter(|state| done(state)) {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

async fn until_removed(path: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

fn listed(count: usize) -> impl Fn(&serde_json::Value) -> bool {
    move |state| state["sessions"].as_array().map(Vec::len) == Some(count)
}

#[tokio::test(start_paused = true)]
async fn opening_a_session_writes_it_to_bar_json() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("bar.json");
    let viewer = BarStateViewer::new(Arc::new(NoopViewer), path.clone());
    let (_events, snapshots) = watch::channel(working());

    viewer
        .opened(info(0, CardSpot::Focus(focus_geometry())), snapshots)
        .await;

    let state = until_written(&path, listed(1)).await;
    assert_eq!(state["version"], 1);
    assert_eq!(state["sessions"][0]["n"], 1);
    assert_eq!(state["sessions"][0]["state"], "working");
    assert_eq!(state["sessions"][0]["in_focus"], true);
}

#[tokio::test(start_paused = true)]
async fn a_snapshot_change_rewrites_the_file() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("bar.json");
    let viewer = BarStateViewer::new(Arc::new(NoopViewer), path.clone());
    let (events, snapshots) = watch::channel(working());
    viewer
        .opened(info(0, CardSpot::Rail(RAIL)), snapshots)
        .await;
    until_written(&path, listed(1)).await;

    events
        .send(SessionSnapshot {
            control: ControlState::HelpRequest {
                reason: "2FA code".into(),
                start_ms: Millis(0),
            },
            ..working()
        })
        .unwrap();

    let state = until_written(&path, |state| state["sessions"][0]["state"] == "needs_you").await;
    assert_eq!(state["sessions"][0]["detail"], "2FA code");
}

#[tokio::test(start_paused = true)]
async fn arranging_marks_the_focus_slot_in_focus() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("bar.json");
    let viewer = BarStateViewer::new(Arc::new(NoopViewer), path.clone());
    let (_first, snapshots) = watch::channel(working());
    viewer
        .opened(info(0, CardSpot::Rail(RAIL)), snapshots)
        .await;
    let (_second, snapshots) = watch::channel(working());
    viewer
        .opened(info(1, CardSpot::Focus(focus_geometry())), snapshots)
        .await;
    until_written(&path, |state| state["sessions"][1]["in_focus"] == true).await;

    viewer
        .arranged(vec![
            (SlotId(0), CardSpot::Focus(focus_geometry())),
            (SlotId(1), CardSpot::Rail(RAIL)),
        ])
        .await;

    let state = until_written(&path, |state| state["sessions"][0]["in_focus"] == true).await;
    assert_eq!(state["sessions"][0]["n"], 1);
    assert_eq!(state["sessions"][1]["in_focus"], false);
}

#[tokio::test(start_paused = true)]
async fn an_ended_session_stays_listed_until_its_card_closes() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("bar.json");
    let viewer = BarStateViewer::new(Arc::new(NoopViewer), path.clone());
    let (events, snapshots) = watch::channel(working());
    viewer
        .opened(info(0, CardSpot::Rail(RAIL)), snapshots)
        .await;
    let (_other, snapshots) = watch::channel(working());
    viewer
        .opened(info(1, CardSpot::Focus(focus_geometry())), snapshots)
        .await;

    events.send(ended()).unwrap();
    let state = until_written(&path, |state| state["sessions"][0]["state"] == "ended").await;
    assert_eq!(state["sessions"][0]["verdict"], "pass");

    viewer.closed(SlotId(0)).await;
    let state = until_written(&path, listed(1)).await;
    assert_eq!(state["sessions"][0]["n"], 2);
}

#[tokio::test(start_paused = true)]
async fn closing_the_last_card_removes_bar_json() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("bar.json");
    let viewer = BarStateViewer::new(Arc::new(NoopViewer), path.clone());
    let (_events, snapshots) = watch::channel(working());
    viewer
        .opened(info(0, CardSpot::Rail(RAIL)), snapshots)
        .await;
    until_written(&path, listed(1)).await;

    viewer.closed(SlotId(0)).await;

    until_removed(&path).await;
}

#[tokio::test(start_paused = true)]
async fn shutting_down_removes_bar_json() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("bar.json");
    let viewer = BarStateViewer::new(Arc::new(NoopViewer), path.clone());
    let (_events, snapshots) = watch::channel(working());
    viewer
        .opened(info(0, CardSpot::Rail(RAIL)), snapshots)
        .await;
    until_written(&path, listed(1)).await;

    viewer.shutdown().await;

    assert!(!path.exists());
}

#[derive(Default)]
struct Calls(Mutex<Vec<&'static str>>);
impl Calls {
    fn push(&self, call: &'static str) -> ViewerFuture<'_> {
        self.0.lock().unwrap().push(call);
        Box::pin(async {})
    }
}
impl Viewer for Calls {
    fn take_commands(&self) -> Option<mpsc::Receiver<ViewerCommand>> {
        Some(mpsc::channel(1).1)
    }
    fn opened(&self, _: SessionInfo, _: watch::Receiver<SessionSnapshot>) -> ViewerFuture<'_> {
        self.push("opened")
    }
    fn closed(&self, _: SlotId) -> ViewerFuture<'_> {
        self.push("closed")
    }
    fn arranged(&self, _: Vec<(SlotId, CardSpot)>) -> ViewerFuture<'_> {
        self.push("arranged")
    }
    fn still(&self, _: SlotId, _: Still) -> ViewerFuture<'_> {
        self.push("still")
    }
    fn shutdown(&self) -> ViewerFuture<'_> {
        self.push("shutdown")
    }
}

#[tokio::test(start_paused = true)]
async fn inner_viewer_calls_are_forwarded() {
    let folder = tempfile::tempdir().unwrap();
    let calls = Arc::new(Calls::default());
    let viewer = BarStateViewer::new(calls.clone(), folder.path().join("bar.json"));
    let (_events, snapshots) = watch::channel(working());

    viewer
        .opened(info(0, CardSpot::Rail(RAIL)), snapshots)
        .await;
    viewer
        .arranged(vec![(SlotId(0), CardSpot::Focus(focus_geometry()))])
        .await;
    let still = Still {
        path: folder.path().join("still.png"),
        taken: chrono::DateTime::from_timestamp(0, 0).unwrap(),
    };
    viewer.still(SlotId(0), still).await;
    viewer.closed(SlotId(0)).await;
    viewer.shutdown().await;

    assert_eq!(
        *calls.0.lock().unwrap(),
        vec!["opened", "arranged", "still", "closed", "shutdown"]
    );
    assert!(viewer.supports_cards());
    assert!(viewer.take_commands().is_some());
}
