use super::{
    CardSpot, SessionInfo, ShowTarget, SlotId, Still, ViewerCommand, end_card,
    focus_card::FocusCard,
    footer_view::{self, FooterButton, Hover, SessionColour},
    rail_card::RailCard,
    rail_panels,
    rail_view::{self, NoteSpot, RailCardView},
};
use crate::{
    ipc::{SessionSnapshot, SessionState},
    slots::{RailShape, Slot},
};
use gtk4::{self as gtk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};
use tokio::sync::mpsc;

mod messages;
pub use messages::run;

struct Card {
    info: SessionInfo,
    snapshot: SessionSnapshot,
    frozen: Option<u64>,
    watcher: Option<glib::JoinHandle<()>>,
    monitor: gtk::gdk::Monitor,
    spot: CardSpot,
    still: Option<Still>,
    texture: Option<gtk::gdk::Texture>,
    surfaces: Surfaces,
}

enum Surfaces {
    Focus(FocusCard),
    Rail(RailCard),
}

impl Surfaces {
    fn new(
        info: &SessionInfo,
        snapshot: &SessionSnapshot,
        frozen: Option<u64>,
        monitor: &gtk::gdk::Monitor,
        spot: CardSpot,
        state: &Rc<RefCell<State>>,
    ) -> Self {
        let slot = info.slot;
        let colour = SessionColour::for_slot(slot);
        let now = chrono::Utc::now();
        let weak = Rc::downgrade(state);
        match spot {
            CardSpot::Focus(geometry) => {
                let body_state = weak.clone();
                let hover_state = weak.clone();
                let view = footer_view::for_session(info, snapshot, now, Hover::Away, frozen);
                let focus = FocusCard::new(
                    monitor,
                    geometry,
                    &view,
                    snapshot,
                    move |action| {
                        if let Some(state) = weak.upgrade() {
                            let data = state.borrow();
                            if let Some(card) = data.cards.get(&slot) {
                                data.activate(card, action);
                            }
                        }
                    },
                    move || {
                        if let Some(state) = body_state.upgrade() {
                            let data = state.borrow();
                            if let Some(card) = data.cards.get(&slot)
                                && let Some(action) = footer_view::body_click(&card.snapshot)
                            {
                                data.activate(card, action);
                            }
                        }
                    },
                    move |hover| {
                        if let Some(state) = hover_state.upgrade()
                            && let Ok(mut data) = state.try_borrow_mut()
                            && let Some(card) = data.cards.get_mut(&slot)
                            && let Surfaces::Focus(focus) = &mut card.surfaces
                            && (hover == Hover::Away || card.snapshot.state == SessionState::Open)
                        {
                            focus.hover = hover;
                            card.draw();
                        }
                    },
                );
                focus.draw(&view, colour);
                Self::Focus(focus)
            }
            CardSpot::Rail(rect) => {
                let hover_state = weak.clone();
                let view = rail_view::for_rail(info, snapshot, now, None, false);
                Self::Rail(RailCard::new(
                    monitor,
                    rect,
                    colour,
                    &view,
                    move || {
                        if let Some(state) = weak.upgrade()
                            && let Ok(data) = state.try_borrow()
                            && let Some(card) = data.cards.get(&slot)
                            && let Surfaces::Rail(rail) = &card.surfaces
                            && !rail.locked
                        {
                            data.show(ShowTarget::Session(card.info.name.clone()));
                        }
                    },
                    move |over| {
                        if let Some(state) = hover_state.upgrade()
                            && State::set_rail_hover(&state, slot, over)
                        {
                            State::visibility(&state);
                        }
                    },
                ))
            }
        }
    }

    fn close(self) {
        match self {
            Self::Focus(focus) => focus.close(),
            Self::Rail(rail) => rail.close(),
        }
    }
}

impl Card {
    fn rearranged(mut self, spot: CardSpot, state: &Rc<RefCell<State>>) -> Self {
        self.surfaces.close();
        self.spot = spot;
        self.surfaces = Surfaces::new(
            &self.info,
            &self.snapshot,
            self.frozen,
            &self.monitor,
            spot,
            state,
        );
        if let Surfaces::Rail(rail) = &mut self.surfaces {
            if let Some(texture) = &self.texture {
                rail.set_texture(texture);
            }
            rail.load_end_frame(&self.snapshot.state);
        }
        self
    }

    fn focus_view(&self, hover: Hover) -> footer_view::FooterView {
        footer_view::for_session(
            &self.info,
            &self.snapshot,
            chrono::Utc::now(),
            hover,
            self.frozen,
        )
    }

    fn draw_focus(&self, focus: &FocusCard) {
        focus.draw(
            &self.focus_view(focus.hover),
            SessionColour::for_slot(self.info.slot),
        );
    }

    fn rail_view(&self, locked: bool) -> RailCardView {
        let mut view = rail_view::for_rail(
            &self.info,
            &self.snapshot,
            chrono::Utc::now(),
            self.still.as_ref().map(|still| still.taken),
            locked,
        );
        if self.snapshot.state.ticks()
            && let Some(seconds) = self.frozen
        {
            view.elapsed = footer_view::duration(seconds);
        }
        view
    }

    fn draw(&self) {
        match &self.surfaces {
            Surfaces::Focus(focus) => self.draw_focus(focus),
            Surfaces::Rail(rail) => rail.draw(&self.rail_view(rail.locked)),
        }
    }

    fn freeze_elapsed(&mut self) {
        if self.snapshot.state == SessionState::Closing && self.frozen.is_none() {
            self.frozen = Some(
                chrono::Utc::now()
                    .signed_duration_since(self.info.opened)
                    .num_seconds()
                    .max(0) as u64,
            );
        }
    }

    fn update(&mut self, snapshot: SessionSnapshot) {
        self.snapshot = snapshot;
        self.freeze_elapsed();
        if let Surfaces::Focus(focus) = &mut self.surfaces
            && let CardSpot::Focus(geometry) = self.spot
        {
            focus.update_end(&self.snapshot, geometry);
            self.draw();
        }
        if let Surfaces::Rail(rail) = &mut self.surfaces {
            rail.load_end_frame(&self.snapshot.state);
        }
    }

    fn set_still(&mut self, still: Still) {
        match gtk::gdk::Texture::from_filename(&still.path) {
            Ok(texture) => {
                if let Surfaces::Rail(rail) = &self.surfaces {
                    rail.set_texture(&texture);
                }
                self.still = Some(still);
                self.texture = Some(texture);
            }
            Err(error) => {
                tracing::debug!(%error, slot = ?self.info.slot, "Could not load the rail still")
            }
        }
    }

    fn set_visible(&mut self, shown: bool, locked: bool) {
        match self.spot {
            CardSpot::Focus(_) => {
                if let Surfaces::Focus(focus) = &mut self.surfaces {
                    let reset = !shown && focus.hover != Hover::Away;
                    if reset {
                        focus.hover = Hover::Away;
                    }
                    focus.set_visible(shown, &self.snapshot);
                    if reset {
                        self.draw();
                    }
                }
            }
            CardSpot::Rail(rect) => {
                let view = self.rail_view(locked);
                if let Surfaces::Rail(rail) = &mut self.surfaces {
                    rail.locked = locked;
                    if !shown {
                        rail.over = false;
                    }
                    rail.draw(&view);
                    rail.window.set_visible(shown);
                    rail.sync_hover(
                        &self.monitor,
                        shown.then(|| {
                            let shape = RailShape::for_width(self.monitor.geometry().width());
                            rail_view::hover_spot(rect, shape)
                        }),
                        view.hover,
                    );
                }
            }
        }
    }

    fn needs_tick(&self) -> bool {
        self.snapshot.state.ticks()
            || (matches!(&self.surfaces, Surfaces::Rail(_))
                && !matches!(self.snapshot.state, SessionState::Ended(_))
                && self.still.is_some())
    }

    fn close(self) {
        if let Some(handle) = self.watcher {
            handle.abort();
        }
        self.surfaces.close();
    }
}

struct State {
    cards: HashMap<SlotId, Card>,
    missing: HashMap<SlotId, glib::JoinHandle<()>>,
    visible: HashSet<String>,
    tick: Option<glib::SourceId>,
    notes: HashMap<String, (NoteSpot, gtk::Window)>,
    commands: Option<mpsc::Sender<ViewerCommand>>,
}
impl State {
    fn activate(&self, card: &Card, action: FooterButton) {
        let name = &card.info.name;
        let command = match action {
            FooterButton::TakeControl => ViewerCommand::TakeControl(name.clone()),
            FooterButton::StepAway => ViewerCommand::StepAway(name.clone()),
            FooterButton::Resume => ViewerCommand::Resume(name.clone()),
            FooterButton::Done => ViewerCommand::Done(name.clone()),
            FooterButton::Dismiss => ViewerCommand::Dismiss(name.clone()),
            FooterButton::OpenVideo | FooterButton::OpenFolder => {
                if let SessionState::Ended(end) = &card.snapshot.state {
                    let path = if action == FooterButton::OpenVideo {
                        &end.verification
                    } else {
                        &end.folder
                    };
                    if let Some(path) = path {
                        end_card::open(path);
                    }
                }
                return;
            }
        };
        if let Some(commands) = &self.commands
            && let Err(error) = commands.try_send(command)
        {
            tracing::warn!(%error, "Session Footer command could not be delivered");
        }
    }

    fn show(&self, target: ShowTarget) {
        if let Some(commands) = &self.commands
            && let Err(error) = commands.try_send(ViewerCommand::Show(target))
        {
            tracing::warn!(%error, "Show command could not be delivered");
        }
    }

    fn set_rail_hover(state: &Rc<RefCell<Self>>, slot: SlotId, over: bool) -> bool {
        if let Ok(mut data) = state.try_borrow_mut()
            && let Some(card) = data.cards.get_mut(&slot)
            && let Surfaces::Rail(rail) = &mut card.surfaces
        {
            rail.over = over;
            true
        } else {
            false
        }
    }

    fn visibility(state: &Rc<RefCell<Self>>) {
        let mut data = state.borrow_mut();
        let data = &mut *data;
        let locked = rail_view::rail_locked(data.cards.values().map(|card| &card.snapshot));
        let mut rails = HashMap::<String, (gtk::gdk::Monitor, Vec<Slot>)>::new();
        for card in data.cards.values() {
            if let CardSpot::Rail(rect) = card.spot
                && data.visible.contains(&card.info.monitor)
            {
                rails
                    .entry(card.info.monitor.clone())
                    .or_insert_with(|| (card.monitor.clone(), Vec::new()))
                    .1
                    .push(rect);
            }
        }
        let visible = &data.visible;
        for card in data.cards.values_mut() {
            card.set_visible(visible.contains(&card.info.monitor), locked);
        }
        let ticking = data
            .cards
            .values()
            .any(|card| visible.contains(&card.info.monitor) && card.needs_tick());
        data.sync_notes(locked, &rails);
        if ticking && data.tick.is_none() {
            let weak = Rc::downgrade(state);
            data.tick =
                Some(glib::timeout_add_local(Duration::from_secs(1), move || {
                    let Some(state) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let data = state.borrow();
                    for card in data.cards.values().filter(|card| {
                        data.visible.contains(&card.info.monitor) && card.needs_tick()
                    }) {
                        card.draw();
                    }
                    glib::ControlFlow::Continue
                }));
        }
        if !ticking && let Some(tick) = data.tick.take() {
            tick.remove();
        }
    }

    fn sync_notes(
        &mut self,
        locked: bool,
        rails: &HashMap<String, (gtk::gdk::Monitor, Vec<Slot>)>,
    ) {
        let spots: HashMap<_, _> = rails
            .iter()
            .filter(|_| locked)
            .filter_map(|(name, (monitor, slots))| {
                let shape = RailShape::for_width(monitor.geometry().width());
                rail_view::locked_note_spot(slots, shape).map(|spot| (name, (monitor, spot)))
            })
            .collect();
        self.notes.retain(|name, (spot, window)| {
            let keep = spots.get(&name).is_some_and(|(_, wanted)| wanted == spot);
            if !keep {
                window.close();
            }
            keep
        });
        for (name, (monitor, spot)) in spots {
            self.notes.entry(name.clone()).or_insert_with(|| {
                let note = rail_panels::locked_surface(monitor, spot);
                note.set_visible(true);
                (spot, note)
            });
        }
    }
}
pub(super) fn surface(
    monitor: &gtk::gdk::Monitor,
    layer: Layer,
    namespace: &str,
    rect: Slot,
) -> gtk::Window {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(layer);
    window.set_namespace(Some(namespace));
    // Slot margins include reserved space, so place surfaces against the full monitor.
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_monitor(Some(monitor));
    window.set_margin(Edge::Left, rect.x);
    window.set_margin(Edge::Top, rect.y);
    window.set_default_size(rect.width, rect.height);
    window.set_resizable(false);
    window
}

pub(super) fn primary_click(activate: impl Fn() + 'static) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_PRIMARY);
    click.connect_released(move |_, _, _, _| activate());
    click
}

fn create(
    info: SessionInfo,
    snapshot: SessionSnapshot,
    state: &Rc<RefCell<State>>,
) -> Option<Card> {
    let monitor = gtk::gdk::Display::default().and_then(|display| {
        let monitors = display.monitors();
        (0..monitors.n_items()).find_map(|n| {
            monitors
                .item(n)
                .and_then(|item| item.downcast::<gtk::gdk::Monitor>().ok())
                .filter(|monitor| monitor.connector().as_deref() == Some(info.monitor.as_str()))
        })
    });
    let Some(monitor) = monitor else {
        tracing::warn!(connector = %info.monitor, "No GTK monitor matches the Session Card; skipping it");
        return None;
    };
    let spot = info.spot;
    let mut card = Card {
        surfaces: Surfaces::new(&info, &snapshot, None, &monitor, spot, state),
        info,
        snapshot,
        frozen: None,
        watcher: None,
        monitor,
        spot,
        still: None,
        texture: None,
    };
    card.freeze_elapsed();
    if let Surfaces::Rail(rail) = &mut card.surfaces {
        rail.load_end_frame(&card.snapshot.state);
    }
    card.draw();
    Some(card)
}

pub fn init() -> Result<(), glib::BoolError> {
    gtk::init()?;
    if !gtk4_layer_shell::is_supported() {
        return Err(glib::bool_error!(
            "Session Footers need a Wayland compositor with layer-shell support"
        ));
    }
    Ok(())
}
