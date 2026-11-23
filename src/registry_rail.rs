use super::*;
use crate::compositor::{Placement, WindowSpot, WorkspacePlacement};
use crate::ipc::ControlState;
use crate::places::Place;
use crate::project::WindowSize;
use crate::rail_stills::{self, StillSchedule};
use crate::slots::{self, Slot};
use crate::viewer::{CardSpot, ShowTarget, SlotId, Still};
use tokio::task::AbortHandle;

#[derive(Default)]
pub(super) struct RailStills {
    schedule: StillSchedule,
    pending: HashMap<SessionName, PendingStill>,
}

struct PendingStill {
    task: AbortHandle,
    done: oneshot::Receiver<()>,
}

impl Registry {
    pub(super) fn free_slot(&self, replaced: Option<&SessionName>) -> Option<SlotId> {
        (0..self.dependencies.cap).map(SlotId).find(|slot| {
            !self.sessions.values().any(|entry| entry.slot == *slot)
                && !self
                    .cards
                    .iter()
                    .any(|(name, card)| card.slot == *slot && Some(name) != replaced)
        })
    }

    pub(super) fn opening_placement(
        &self,
        workspace: &WorkspacePlacement,
        place: Place,
        size: Option<WindowSize>,
    ) -> Result<(Placement, CardSpot), SessionError> {
        let focus = workspace.layout.focus;
        let size = match size {
            None => (focus.width, focus.height),
            Some(size) => {
                let width = i32::try_from(size.width.get()).map_err(|_| {
                    SessionError::Refused(
                        "WindowSize width is too large. Reduce it and retry".into(),
                    )
                })?;
                let height = i32::try_from(size.height.get()).map_err(|_| {
                    SessionError::Refused(
                        "WindowSize height is too large. Reduce it and retry".into(),
                    )
                })?;
                if width > focus.width || height > focus.height {
                    return Err(SessionError::Refused(
                        "The Project WindowSize exceeds its Session Card slot. Reduce WindowSize or use a larger monitor".into(),
                    ));
                }
                (width, height)
            }
        };
        let spot = self.card_spot(workspace, place, size).ok_or_else(|| {
            SessionError::Refused(
                "The monitor has no room for the Agents Workspace. Use a larger monitor or reduce session_cap in the keek config".into(),
            )
        })?;
        let geometry = match spot {
            CardSpot::Focus(card) => card,
            CardSpot::Rail(_) => slots::geometry(focus, Some(size)),
        };
        Ok((
            Placement {
                geometry,
                origin: workspace.origin,
                monitor: workspace.monitor.clone(),
                initial: match spot {
                    CardSpot::Rail(_) if self.dependencies.viewer.supports_cards() => {
                        WindowSpot::Hidden
                    }
                    CardSpot::Focus(_) | CardSpot::Rail(_) => {
                        shown(geometry.window, workspace.origin)
                    }
                },
            },
            spot,
        ))
    }

    fn card_spot(
        &self,
        workspace: &WorkspacePlacement,
        place: Place,
        size: (i32, i32),
    ) -> Option<CardSpot> {
        let layout = &workspace.layout;
        match place {
            Place::Focus => {
                let body = if self.places.has_rail() {
                    layout.focus
                } else {
                    layout.alone
                };
                Some(CardSpot::Focus(slots::geometry(body, Some(size))))
            }
            Place::Rail(index) => layout.rail.get(index).copied().map(CardSpot::Rail),
        }
    }

    pub(super) async fn arrange(&mut self) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        let supports_cards = self.dependencies.viewer.supports_cards();
        let mut cards = Vec::new();
        for (name, place) in self.places.occupants() {
            if let Some(entry) = self.sessions.get(name)
                && let Some(spot) = self.card_spot(workspace, place, entry.window_size)
            {
                let wanted = match spot {
                    CardSpot::Focus(card) => {
                        self.stills.schedule.forget(name);
                        shown(card.window, workspace.origin)
                    }
                    CardSpot::Rail(_) if supports_cards => WindowSpot::Hidden,
                    CardSpot::Rail(_) => shown(
                        slots::geometry(workspace.layout.focus, Some(entry.window_size)).window,
                        workspace.origin,
                    ),
                };
                entry
                    .window
                    .move_to(wanted, &*self.dependencies.compositor)
                    .await;
                cards.push((entry.slot, spot));
            } else if let Some(card) = self.cards.get(name)
                && let Some(spot) = self.card_spot(workspace, place, card.window_size)
            {
                cards.push((card.slot, spot));
            }
        }
        self.dependencies.viewer.arranged(cards).await;
    }

    pub(super) async fn show(&mut self, target: ShowTarget) {
        if self
            .sessions
            .values()
            .any(|entry| matches!(entry.events.borrow().control, ControlState::Takeover { .. }))
        {
            tracing::info!("Cannot show another Agent Session during a Takeover");
            return;
        }
        let name = match target {
            ShowTarget::Session(name) => Some(name),
            ShowTarget::Number(number) => number.checked_sub(1).and_then(|index| {
                let slot = SlotId(index);
                self.sessions
                    .iter()
                    .find_map(|(name, entry)| (entry.slot == slot).then_some(name))
                    .or_else(|| {
                        self.cards
                            .iter()
                            .find_map(|(name, card)| (card.slot == slot).then_some(name))
                    })
                    .cloned()
            }),
        };
        if let Some(name) = name
            && self.places.show(&name)
        {
            self.arrange().await;
        }
    }

    pub(super) async fn leave_card(
        &mut self,
        name: &SessionName,
        slot: SlotId,
        frame: Option<PathBuf>,
    ) {
        self.places.leave(name);
        self.close_card(name, slot, frame).await;
        self.arrange().await;
    }

    pub(super) async fn close_card(
        &mut self,
        name: &SessionName,
        slot: SlotId,
        frame: Option<PathBuf>,
    ) {
        self.stills.schedule.forget(name);
        if let Some(pending) = self.stills.pending.remove(name) {
            pending.task.abort();
            // An abort cannot stop a rename already in spawn_blocking, so wait before deleting.
            let _ = pending.done.await;
        }
        self.dependencies.viewer.closed(slot).await;
        let still = rail_stills::still_path(&self.dependencies.frames_dir, name);
        let result = tokio::task::spawn_blocking(move || {
            let next = still.with_extension("next.jpg");
            for path in frame.into_iter().chain([still, next]) {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => tracing::warn!(%error, path = %path.display(), "could not remove Session Card image"),
                }
            }
        }).await;
        if let Err(error) = result {
            tracing::warn!(%error, "could not finish removing Session Card images");
        }
    }

    pub fn take_stills(&mut self) {
        if !self.dependencies.viewer.supports_cards() {
            return;
        }
        let Some(workspace) = &self.workspace else {
            return;
        };
        self.stills.pending.retain(|_, pending| {
            matches!(
                pending.done.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            )
        });
        let mut names = Vec::new();
        let mut windows = HashMap::new();
        for (name, place) in self.places.occupants() {
            if let Place::Rail(index) = place
                && let Some(entry) = self.sessions.get(name)
                && matches!(
                    entry.events.borrow().state,
                    SessionState::Opening | SessionState::Open
                )
                && let Some(address) = entry.window.address()
                && let Some(card) = workspace.layout.rail.get(index)
            {
                let scale = f64::from(card.width - 4) / f64::from(entry.window_size.0);
                names.push(name.clone());
                windows.insert(
                    name.clone(),
                    (entry.slot, address, scale, entry.still_warning.clone()),
                );
            }
        }
        for name in self
            .stills
            .schedule
            .due(self.dependencies.clock.now(), &names)
        {
            if self.stills.pending.contains_key(&name) {
                continue;
            }
            let Some((slot, address, scale, warning)) = windows.remove(&name) else {
                continue;
            };
            let path = rail_stills::still_path(&self.dependencies.frames_dir, &name);
            let frames_dir = self.dependencies.frames_dir.clone();
            let runner = self.dependencies.runner.clone();
            let viewer = self.dependencies.viewer.clone();
            let clock = self.dependencies.clock.clone();
            let (complete, done) = oneshot::channel();
            let complete = Arc::new(complete);
            let session = name.clone();
            let task = self.background.spawn(async move {
                let result = async {
                    tokio::task::spawn_blocking(move || {
                        crate::session_frame::prepare_directory(&frames_dir)
                    })
                    .await?
                    .map_err(|error| rail_stills::StillError::Save(path.clone(), error))?;
                    rail_stills::take(&*runner, &address, scale, &path, complete.clone()).await
                }
                .await;
                match result {
                    Ok(()) => {
                        viewer
                            .still(
                                slot,
                                Still {
                                    path,
                                    taken: clock.local_time().with_timezone(&chrono::Utc),
                                },
                            )
                            .await
                    }
                    Err(error) if warning.set(()).is_ok() => {
                        tracing::warn!(%error, %session, "could not take a rail still")
                    }
                    Err(error) => tracing::debug!(%error, %session, "could not take a rail still"),
                }
                drop(complete);
            });
            self.stills
                .pending
                .insert(name, PendingStill { task, done });
        }
    }
}

fn shown(window: Slot, (x, y): (i32, i32)) -> WindowSpot {
    WindowSpot::Shown {
        x: window.x + x,
        y: window.y + y,
    }
}
