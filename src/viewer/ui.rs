use super::{
    SessionInfo, SlotId, ViewerCommand, ViewerMessage, end_card,
    footer_content::FooterContent,
    footer_view::{self, FooterButton},
    paused_card,
};
use crate::{
    ipc::{SessionSnapshot, SessionState},
    slots::Slot,
};
use gtk4::{self as gtk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

struct Card {
    info: SessionInfo,
    snapshot: SessionSnapshot,
    frozen: Option<u64>,
    hover: footer_view::Hover,
    footer: gtk::Window,
    surface: gtk::Window,
    paused: gtk::Window,
    body: gtk::Overlay,
    content: FooterContent,
    watcher: Option<glib::JoinHandle<()>>,
    end_card: Option<end_card::EndCard>,
}
impl Card {
    fn draw(&self) {
        let view = footer_view::for_session(
            &self.info,
            &self.snapshot,
            chrono::Utc::now(),
            self.hover,
            self.frozen,
        );
        let mut classes = vec![
            "footer",
            footer_view::SessionColour::for_slot(self.info.slot).css_class(),
        ];
        if let Some(class) = view.label.css_class() {
            classes.push(class);
        }
        self.footer.set_css_classes(&classes);
        classes[0] = "hover-surface";
        self.surface.set_css_classes(&classes);
        self.content.draw(&view);
    }
    fn close(self) {
        if let Some(handle) = self.watcher {
            handle.abort();
        }
        self.footer.close();
        self.surface.close();
        self.paused.close();
    }
}
struct State {
    cards: HashMap<SlotId, Card>,
    missing: HashMap<SlotId, glib::JoinHandle<()>>,
    visible: HashSet<String>,
    tick: Option<glib::SourceId>,
    commands: Option<tokio::sync::mpsc::Sender<ViewerCommand>>,
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

    fn visibility(state: &Rc<RefCell<Self>>) {
        let mut data = state.borrow_mut();
        let visible = data.visible.clone();
        for card in data.cards.values_mut() {
            let shown = visible.contains(&card.info.monitor);
            if !shown && card.hover != footer_view::Hover::Away {
                card.hover = footer_view::Hover::Away;
                card.draw();
            }
            let paused = shown && footer_view::is_paused(&card.snapshot);
            if paused && !card.paused.is_visible() {
                card.footer.set_visible(false);
            }
            // Mapping the footer last keeps its overlapping edge above the card surfaces.
            card.surface.set_visible(shown);
            card.paused.set_visible(paused);
            card.footer.set_visible(shown);
        }
        let ticking = data
            .cards
            .values()
            .any(|card| visible.contains(&card.info.monitor) && card.snapshot.state.ticks());
        if ticking && data.tick.is_none() {
            let weak = Rc::downgrade(state);
            data.tick = Some(glib::timeout_add_local(Duration::from_secs(1), move || {
                let Some(state) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                for card in state
                    .borrow()
                    .cards
                    .values()
                    .filter(|card| card.footer.is_visible() && card.snapshot.state.ticks())
                {
                    card.draw();
                }
                glib::ControlFlow::Continue
            }));
        }
        if !ticking && let Some(tick) = data.tick.take() {
            tick.remove();
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

fn primary_click(activate: impl Fn() + 'static) -> gtk::GestureClick {
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
    let footer = surface(&monitor, Layer::Top, "keek-footer", info.placement.footer);
    let surface = surface(&monitor, Layer::Bottom, "keek-footer", info.placement.body);
    let paused = paused_card::new(info.placement, &monitor);
    let body = gtk::Overlay::new();
    let edge = gtk::Box::new(gtk::Orientation::Vertical, 0);
    edge.add_css_class("card-edge");
    edge.set_can_target(false);
    body.add_overlay(&edge);
    surface.set_child(Some(&body));
    let view = footer_view::for_session(
        &info,
        &snapshot,
        chrono::Utc::now(),
        footer_view::Hover::Away,
        None,
    );
    let weak = Rc::downgrade(state);
    let slot = info.slot;
    let content = FooterContent::new(&view, move |action| {
        if let Some(state) = weak.upgrade() {
            let data = state.borrow();
            if let Some(card) = data.cards.get(&slot) {
                data.activate(card, action);
            }
        }
    });
    footer.set_child(Some(&content.row));
    let weak = Rc::downgrade(state);
    let activate_body = move || {
        if let Some(state) = weak.upgrade() {
            let data = state.borrow();
            if let Some(card) = data.cards.get(&slot)
                && let Some(action) = footer_view::body_click(&card.snapshot)
            {
                data.activate(card, action);
            }
        }
    };
    surface.add_controller(primary_click(activate_body.clone()));
    paused.add_controller(primary_click(activate_body));
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(state);
    let slot = info.slot;
    motion.connect_enter(move |_, _, _| {
        if let Some(state) = weak.upgrade()
            && let Ok(mut data) = state.try_borrow_mut()
            && let Some(card) = data.cards.get_mut(&slot)
        {
            if card.snapshot.state != SessionState::Open {
                return;
            }
            card.hover = footer_view::Hover::Over;
            card.draw();
        }
    });
    let weak = Rc::downgrade(state);
    motion.connect_leave(move |_| {
        if let Some(state) = weak.upgrade()
            && let Ok(mut data) = state.try_borrow_mut()
            && let Some(card) = data.cards.get_mut(&slot)
        {
            card.hover = footer_view::Hover::Away;
            card.draw();
        }
    });
    surface.add_controller(motion);
    let frozen = (snapshot.state == SessionState::Closing).then(|| {
        chrono::Utc::now()
            .signed_duration_since(info.opened)
            .num_seconds()
            .max(0) as u64
    });
    let end_card = end_card::EndCard::new(&snapshot, info.placement);
    let card = Card {
        info,
        snapshot,
        frozen,
        hover: footer_view::Hover::Away,
        footer,
        surface,
        paused,
        body,
        content,
        watcher: None,
        end_card,
    };
    card.body
        .set_child(card.end_card.as_ref().map(|end| &end.overlay));
    if card.end_card.is_some() {
        card.footer.set_layer(Layer::Bottom);
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
pub fn run(
    mut receiver: tokio::sync::mpsc::Receiver<ViewerMessage>,
    handle: tokio::runtime::Handle,
) {
    super::style::install_style();
    let main_loop = glib::MainLoop::new(None, false);
    let state = Rc::new(RefCell::new(State {
        cards: HashMap::new(),
        missing: HashMap::new(),
        visible: HashSet::new(),
        tick: None,
        commands: None,
    }));
    let (workspace_tx, mut workspace_rx) = tokio::sync::mpsc::channel(8);
    let monitor_task = handle.spawn(super::events::watch(workspace_tx));
    let workspace_state = state.clone();
    let workspace_watcher = glib::spawn_future_local(async move {
        while let Some(names) = workspace_rx.recv().await {
            workspace_state.borrow_mut().visible = names;
            State::visibility(&workspace_state);
        }
    });
    let message_state = state.clone();
    let quit = main_loop.clone();
    let message_watcher = glib::spawn_future_local(async move {
        while let Some(message) = receiver.recv().await {
            match message {
                ViewerMessage::Commands(commands) => {
                    message_state.borrow_mut().commands = Some(commands);
                }
                ViewerMessage::Opened(info, mut snapshots) => {
                    let slot = info.slot;
                    let name = info.name.clone();
                    let snapshot = snapshots.borrow_and_update().clone();
                    if let Some(old) = message_state.borrow_mut().missing.remove(&slot) {
                        old.abort();
                    }
                    if let Some(old) = message_state.borrow_mut().cards.remove(&slot) {
                        old.close();
                    }
                    let Some(card) = create(info, snapshot, &message_state) else {
                        let commands = message_state.borrow().commands.clone();
                        if let Some(commands) = commands {
                            let task = super::missing_card::watch(snapshots, name, commands);
                            message_state.borrow_mut().missing.insert(slot, task);
                        }
                        continue;
                    };
                    message_state.borrow_mut().cards.insert(slot, card);
                    let weak = Rc::downgrade(&message_state);
                    let watcher = glib::spawn_future_local(async move {
                        while snapshots.changed().await.is_ok() {
                            let snapshot = snapshots.borrow_and_update().clone();
                            let Some(state) = weak.upgrade() else {
                                return;
                            };
                            if let Some(card) = state.borrow_mut().cards.get_mut(&slot) {
                                if snapshot.state == SessionState::Closing && card.frozen.is_none()
                                {
                                    card.frozen = Some(
                                        chrono::Utc::now()
                                            .signed_duration_since(card.info.opened)
                                            .num_seconds()
                                            .max(0) as u64,
                                    );
                                }
                                end_card::update_slot(
                                    &mut card.end_card,
                                    &card.body,
                                    &snapshot,
                                    card.info.placement,
                                );
                                if card.end_card.is_some() {
                                    card.footer.set_layer(Layer::Bottom);
                                }
                                card.snapshot = snapshot;
                                card.draw();
                            }
                            State::visibility(&state);
                        }
                    });
                    if let Some(card) = message_state.borrow_mut().cards.get_mut(&slot) {
                        card.watcher = Some(watcher);
                    }
                    State::visibility(&message_state);
                }
                ViewerMessage::Closed(slot) => {
                    if let Some(task) = message_state.borrow_mut().missing.remove(&slot) {
                        task.abort();
                    }
                    if let Some(card) = message_state.borrow_mut().cards.remove(&slot) {
                        card.close();
                    }
                    State::visibility(&message_state);
                }
                ViewerMessage::Shutdown => break,
            }
        }
        quit.quit();
    });
    main_loop.run();
    message_watcher.abort();
    workspace_watcher.abort();
    monitor_task.abort();
    let mut state = state.borrow_mut();
    if let Some(tick) = state.tick.take() {
        tick.remove();
    }
    for (_, card) in state.cards.drain() {
        card.close();
    }
    for (_, task) in state.missing.drain() {
        task.abort();
    }
}
