use super::*;
use crate::compositor::{WindowAddress, WindowSpot};
use crate::slots::Slot;
use crate::viewer::{CardSpot, ShowTarget, SlotId, ViewerCommand};

#[derive(Default)]
struct RailViewer {
    opened: Mutex<Vec<crate::viewer::SessionInfo>>,
    arranged: Mutex<Vec<Vec<(SlotId, CardSpot)>>>,
    stills: Mutex<Vec<(SlotId, PathBuf)>>,
}
impl crate::viewer::Viewer for RailViewer {
    fn opened(
        &self,
        info: crate::viewer::SessionInfo,
        _: watch::Receiver<SessionSnapshot>,
    ) -> crate::viewer::ViewerFuture<'_> {
        Box::pin(async move { self.opened.lock().unwrap().push(info) })
    }
    fn closed(&self, _: SlotId) -> crate::viewer::ViewerFuture<'_> {
        Box::pin(async {})
    }
    fn arranged(&self, cards: Vec<(SlotId, CardSpot)>) -> crate::viewer::ViewerFuture<'_> {
        Box::pin(async move { self.arranged.lock().unwrap().push(cards) })
    }
    fn still(&self, slot: SlotId, still: crate::viewer::Still) -> crate::viewer::ViewerFuture<'_> {
        Box::pin(async move { self.stills.lock().unwrap().push((slot, still.path)) })
    }
}

fn rail_harness() -> (Harness, Arc<RailViewer>) {
    let mut h = Harness::new(4);
    let viewer = Arc::new(RailViewer::default());
    h.registry.dependencies.viewer = viewer.clone();
    (h, viewer)
}

fn focus() -> Slot {
    reference_workspace(4).layout.focus
}

fn rail(index: usize) -> Slot {
    reference_workspace(4).layout.rail[index]
}

fn focused() -> CardSpot {
    CardSpot::Focus(crate::slots::geometry(focus(), None))
}

fn last_arrangement(viewer: &RailViewer) -> Vec<(SlotId, CardSpot)> {
    let mut cards = viewer.arranged.lock().unwrap().last().cloned().unwrap();
    cards.sort_by_key(|(slot, _)| slot.0);
    cards
}

fn moves_since(h: &Harness, from: usize) -> Vec<(WindowAddress, WindowSpot)> {
    let mut moves = h.compositor.2.lock().unwrap()[from..].to_vec();
    moves.sort_by(|a, b| a.0.0.cmp(&b.0.0));
    moves
}

fn shown_at_focus() -> WindowSpot {
    WindowSpot::Shown { x: 24, y: 50 }
}

#[tokio::test]
async fn four_sessions_fill_the_focus_slot_and_the_rail_and_a_fifth_is_refused() {
    let (mut h, viewer) = rail_harness();
    for name in ["one", "two", "three", "four"] {
        h.open(h.worktree(name)).await;
    }
    assert_eq!(
        last_arrangement(&viewer),
        vec![
            (SlotId(0), focused()),
            (SlotId(1), CardSpot::Rail(rail(0))),
            (SlotId(2), CardSpot::Rail(rail(1))),
            (SlotId(3), CardSpot::Rail(rail(2))),
        ]
    );
    let response = h
        .request(Request::OpenSession {
            project: "app".into(),
            worktree: h.worktree("five"),
            goal: "goal".into(),
            checks: vec![],
        })
        .await;
    assert!(refused(response).contains("cap 4"));
}

#[tokio::test]
async fn one_session_spans_the_workspace_with_its_window_at_the_focus_slot_size() {
    let (mut h, viewer) = rail_harness();
    h.open(h.worktree("one")).await;
    let info = viewer.opened.lock().unwrap()[0].clone();
    let CardSpot::Focus(card) = info.spot else {
        panic!("a lone session is not in focus: {:?}", info.spot);
    };
    assert_eq!(card.body, reference_workspace(4).layout.alone);
    assert_eq!(card.window, Slot { x: 309, ..focus() });
    assert_eq!(info.placement.window.width, focus().width);
    assert_eq!(info.placement.window.height, focus().height);
}

#[tokio::test]
async fn a_session_opening_in_the_rail_opens_at_the_focus_slot_size_out_of_sight() {
    let (mut h, viewer) = rail_harness();
    h.open(h.worktree("one")).await;
    let before = h.compositor.2.lock().unwrap().len();
    h.open(h.worktree("two")).await;
    let second = viewer.opened.lock().unwrap()[1].clone();
    assert_eq!(second.spot, CardSpot::Rail(rail(0)));
    assert_eq!(second.placement.window, focus());
    assert_eq!(
        h.controls.placements.lock().unwrap()[1].initial,
        WindowSpot::Hidden
    );
    assert_eq!(
        moves_since(&h, before),
        vec![(WindowAddress("0xa0".into()), shown_at_focus())]
    );
}

#[tokio::test]
async fn the_focused_session_opens_shown_at_its_window_position() {
    let (mut h, _viewer) = rail_harness();
    h.open(h.worktree("one")).await;
    assert_eq!(
        h.controls.placements.lock().unwrap()[0].initial,
        WindowSpot::Shown { x: 309, y: 50 }
    );
}

#[tokio::test]
async fn clicking_a_rail_card_swaps_its_session_with_the_focused_one() {
    let (mut h, viewer) = rail_harness();
    h.open(h.worktree("one")).await;
    let second = h.open(h.worktree("two")).await;
    let before = h.compositor.2.lock().unwrap().len();
    h.registry
        .control(ViewerCommand::Show(ShowTarget::Session(second.clone())))
        .await;
    assert_eq!(
        moves_since(&h, before),
        vec![
            (WindowAddress("0xa0".into()), WindowSpot::Hidden),
            (WindowAddress("0xa1".into()), shown_at_focus()),
        ]
    );
    assert_eq!(
        last_arrangement(&viewer),
        vec![(SlotId(0), CardSpot::Rail(rail(0))), (SlotId(1), focused()),]
    );
}

#[tokio::test]
async fn a_shortcut_number_shows_the_session_with_that_number() {
    let (mut h, viewer) = rail_harness();
    for name in ["one", "two", "three"] {
        h.open(h.worktree(name)).await;
    }
    h.registry
        .control(ViewerCommand::Show(ShowTarget::Number(3)))
        .await;
    assert_eq!(
        last_arrangement(&viewer),
        vec![
            (SlotId(0), CardSpot::Rail(rail(1))),
            (SlotId(1), CardSpot::Rail(rail(0))),
            (SlotId(2), focused()),
        ]
    );
    let count = viewer.arranged.lock().unwrap().len();
    h.registry
        .control(ViewerCommand::Show(ShowTarget::Number(4)))
        .await;
    assert_eq!(viewer.arranged.lock().unwrap().len(), count);
}

/// Answers the host calls a Takeover makes, so the controller reaches the Takeover state.
struct TakeoverHost(Runner);
impl ProcessRunner for TakeoverHost {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        if program == "hyprctl" {
            // Done hands focus back to the developer's last window, so the host needs one.
            let reply = match args.first().copied() {
                Some("eval") => "ok",
                Some("clients") => r#"[{"address":"0xabc","class":"editor","focusHistoryID":0}]"#,
                Some("activewindow") => r#"{"address":"0xabc"}"#,
                _ => "[]",
            };
            return Box::pin(async move { Ok(reply.into()) });
        }
        self.0.output(program, args, cwd)
    }
}

#[tokio::test]
async fn a_takeover_locks_the_rail_until_done() {
    let (mut h, _viewer) = rail_harness();
    h.registry.dependencies.runner =
        Arc::new(TakeoverHost(Runner(h.directory.path().join("videos"))));
    let first = h.open(h.worktree("one")).await;
    let second = h.open(h.worktree("two")).await;
    let mut events = h.registry.sessions.get(&first).unwrap().events.clone();
    h.registry
        .control(ViewerCommand::TakeControl(first.clone()))
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !matches!(
            events.borrow_and_update().control,
            crate::ipc::ControlState::Takeover { .. }
        ) {
            events.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let before = h.compositor.2.lock().unwrap().len();
    h.registry
        .control(ViewerCommand::Show(ShowTarget::Session(second.clone())))
        .await;
    h.registry
        .control(ViewerCommand::Show(ShowTarget::Number(2)))
        .await;
    assert_eq!(h.compositor.2.lock().unwrap().len(), before);
    assert_eq!(
        h.registry.places.place(&first),
        Some(crate::places::Place::Focus)
    );
    h.registry.control(ViewerCommand::Done(first.clone())).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while matches!(
            events.borrow_and_update().control,
            crate::ipc::ControlState::Takeover { .. }
        ) {
            events.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    h.registry
        .control(ViewerCommand::Show(ShowTarget::Session(second.clone())))
        .await;
    assert_eq!(
        h.registry.places.place(&second),
        Some(crate::places::Place::Focus)
    );
}

#[tokio::test]
async fn dismissing_the_focused_end_card_brings_in_the_oldest_running_rail_session() {
    let (mut h, viewer) = rail_harness();
    let first = h.open(h.worktree("one")).await;
    h.open(h.worktree("two")).await;
    h.open(h.worktree("three")).await;
    h.finish(first.clone()).await;
    assert_eq!(last_arrangement(&viewer)[0], (SlotId(0), focused()));
    let before = h.compositor.2.lock().unwrap().len();
    h.registry.control(ViewerCommand::Dismiss(first)).await;
    assert_eq!(
        last_arrangement(&viewer),
        vec![(SlotId(1), focused()), (SlotId(2), CardSpot::Rail(rail(0))),]
    );
    assert_eq!(
        moves_since(&h, before),
        vec![(WindowAddress("0xa1".into()), shown_at_focus())]
    );
}

#[tokio::test]
async fn clicking_an_ended_rail_card_brings_its_end_card_to_focus() {
    let (mut h, viewer) = rail_harness();
    h.open(h.worktree("one")).await;
    let second = h.open(h.worktree("two")).await;
    h.finish(second.clone()).await;
    let before = h.compositor.2.lock().unwrap().len();
    h.registry
        .control(ViewerCommand::Show(ShowTarget::Session(second)))
        .await;
    assert_eq!(
        last_arrangement(&viewer),
        vec![(SlotId(0), CardSpot::Rail(rail(0))), (SlotId(1), focused()),]
    );
    assert_eq!(
        moves_since(&h, before),
        vec![(WindowAddress("0xa0".into()), WindowSpot::Hidden)]
    );
}

#[tokio::test]
async fn a_new_session_replaces_the_oldest_ended_card_when_no_place_is_empty() {
    let (mut h, viewer) = rail_harness();
    let names = [
        h.open(h.worktree("one")).await,
        h.open(h.worktree("two")).await,
        h.open(h.worktree("three")).await,
        h.open(h.worktree("four")).await,
    ];
    h.finish(names[2].clone()).await;
    h.finish(names[1].clone()).await;
    h.open(h.worktree("five")).await;
    let fifth = viewer.opened.lock().unwrap()[4].clone();
    assert_eq!(fifth.spot, CardSpot::Rail(rail(1)));
    assert!(!h.registry.cards.contains_key(&names[2]));
    assert!(h.registry.cards.contains_key(&names[1]));
}

/// Answers `hyprctl clients -j` with each fake Session Window's stable id, and writes a still for grim.
struct StillHost {
    inner: Runner,
    grim: Mutex<Vec<String>>,
}
impl ProcessRunner for StillHost {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        match program {
            "hyprctl" if args == ["clients", "-j"] => Box::pin(async {
                Ok(r#"[{"address":"0xa0","stableId":"s0"},{"address":"0xa1","stableId":"s1"},{"address":"0xa2","stableId":"s2"}]"#.into())
            }),
            "grim" => Box::pin(async move {
                self.grim.lock().unwrap().push(args[1].to_owned());
                std::fs::write(args.last().unwrap(), b"jpeg").unwrap();
                Ok(String::new())
            }),
            _ => self.inner.output(program, args, cwd),
        }
    }
}

#[tokio::test]
async fn stills_are_taken_for_rail_sessions_only_and_stay_out_of_the_evidence() {
    let (mut h, viewer) = rail_harness();
    let host = Arc::new(StillHost {
        inner: Runner(h.directory.path().join("videos")),
        grim: Mutex::new(Vec::new()),
    });
    h.registry.dependencies.runner = host.clone();
    let names = [
        h.open(h.worktree("one")).await,
        h.open(h.worktree("two")).await,
        h.open(h.worktree("three")).await,
    ];
    let driver_calls = h.calls.lock().unwrap().len();
    h.registry.take_stills();
    while let Some(task) = h.registry.background.join_next().await {
        task.unwrap();
    }
    let mut grim = host.grim.lock().unwrap().clone();
    grim.sort();
    assert_eq!(grim, ["s1", "s2"]);
    let mut stills = viewer.stills.lock().unwrap().clone();
    stills.sort_by_key(|(slot, _)| slot.0);
    assert_eq!(
        stills.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
        [SlotId(1), SlotId(2)]
    );
    for (_, path) in &stills {
        assert!(
            path.starts_with(&h.registry.dependencies.frames_dir),
            "{path:?}"
        );
        assert!(path.exists());
        for name in &names {
            assert!(!path.starts_with(h.registry.dependencies.sessions_dir.join(name.to_string())));
        }
    }
    assert_eq!(h.calls.lock().unwrap().len(), driver_calls);
    h.registry.take_stills();
    assert!(h.registry.background.is_empty());
}

/// Answers `hyprctl clients -j` like StillHost, but grim never finishes.
struct HangingStillHost(Runner);
impl ProcessRunner for HangingStillHost {
    fn output<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        cwd: Option<&'a Path>,
    ) -> crate::process::ProcessFuture<'a> {
        match program {
            "hyprctl" if args == ["clients", "-j"] => {
                Box::pin(async { Ok(r#"[{"address":"0xa1","stableId":"s1"}]"#.into()) })
            }
            "grim" => Box::pin(std::future::pending()),
            _ => self.0.output(program, args, cwd),
        }
    }
}

#[tokio::test]
async fn dismissing_a_card_does_not_wait_for_a_still_that_hangs() {
    let (mut h, _viewer) = rail_harness();
    h.registry.dependencies.runner =
        Arc::new(HangingStillHost(Runner(h.directory.path().join("videos"))));
    h.open(h.worktree("one")).await;
    let second = h.open(h.worktree("two")).await;
    h.registry.take_stills();
    tokio::task::yield_now().await;
    h.finish(second.clone()).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        h.registry.control(ViewerCommand::Dismiss(second.clone())),
    )
    .await
    .expect("dismiss waited for the hanging still");
    assert!(!h.registry.cards.contains_key(&second));
}

#[tokio::test]
async fn without_session_cards_every_window_stays_on_the_agents_workspace_and_no_still_is_taken() {
    let mut h = Harness::new(4);
    h.registry.dependencies.viewer = Arc::new(crate::viewer::NoopViewer);
    let host = Arc::new(StillHost {
        inner: Runner(h.directory.path().join("videos")),
        grim: Mutex::new(Vec::new()),
    });
    h.registry.dependencies.runner = host.clone();
    h.open(h.worktree("one")).await;
    h.open(h.worktree("two")).await;
    assert!(matches!(
        h.controls.placements.lock().unwrap()[1].initial,
        WindowSpot::Shown { .. }
    ));
    assert!(
        !h.compositor
            .2
            .lock()
            .unwrap()
            .iter()
            .any(|(_, spot)| *spot == WindowSpot::Hidden)
    );
    h.registry.take_stills();
    assert!(h.registry.background.is_empty());
    assert!(host.grim.lock().unwrap().is_empty());
}
