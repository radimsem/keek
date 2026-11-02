use super::{SessionInfo, SlotId, ViewerMessage, footer_view, theme};
use crate::ipc::{SessionSnapshot, SessionState};
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
    state_label: gtk::Label,
    detail: gtk::Label,
    checked: gtk::Box,
    elapsed_label: gtk::Label,
    elapsed: gtk::Label,
    button: gtk::Button,
    watcher: Option<glib::JoinHandle<()>>,
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
        self.button.set_label("Take control");
        self.state_label.set_text(view.label.text());
        self.detail.set_text(&view.detail);
        self.elapsed_label.set_text(view.elapsed_label);
        self.elapsed.set_text(&view.elapsed);
        self.state_label
            .set_css_classes(&[if view.label == footer_view::StateLabel::ViewOnly {
                "hint"
            } else {
                "dim"
            }]);
        if view.label == footer_view::StateLabel::ViewOnly {
            self.button.add_css_class("hinted");
        } else {
            self.button.remove_css_class("hinted");
        }
        while let Some(child) = self.checked.first_child() {
            self.checked.remove(&child);
        }
        let Some(checked) = view.checked else {
            self.checked.set_visible(false);
            return;
        };
        self.checked.set_visible(true);
        self.checked.set_tooltip_text(Some(&checked.accessible));
        self.checked
            .update_property(&[gtk::accessible::Property::Label(&checked.accessible)]);
        let title = label("Checked", "dim");
        let value = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        value.append(&label(&checked.count, "value"));
        for kind in checked.ticks {
            let tick = label(
                if kind == footer_view::Tick::Skipped {
                    "−"
                } else {
                    ""
                },
                kind.css_class(),
            );
            tick.add_css_class("tick");
            value.append(&tick);
        }
        if checked.overflow > 0 {
            value.append(&label(&format!("+{}", checked.overflow), "dim"));
        }
        self.checked.append(&title);
        self.checked.append(&value);
    }
    fn close(self) {
        if let Some(handle) = self.watcher {
            handle.abort();
        }
        self.footer.close();
        self.surface.close();
    }
}
struct State {
    cards: HashMap<SlotId, Card>,
    visible: HashSet<String>,
    tick: Option<glib::SourceId>,
}
impl State {
    fn visibility(state: &Rc<RefCell<Self>>) {
        let mut data = state.borrow_mut();
        let visible = data.visible.clone();
        for card in data.cards.values_mut() {
            let shown = visible.contains(&card.info.monitor);
            if !shown && card.hover != footer_view::Hover::Away {
                card.hover = footer_view::Hover::Away;
                card.draw();
            }
            card.footer.set_visible(shown);
            card.surface.set_visible(shown);
        }
        let ticking = data
            .cards
            .values()
            .any(|card| visible.contains(&card.info.monitor));
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
                    .filter(|card| card.footer.is_visible())
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
fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class(class);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}
fn column(title: &str, value: &str, width: i32) -> (gtk::Box, gtk::Label, gtk::Label) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_size_request(width, -1);
    let title = label(title, "dim");
    let value = label(value, "value");
    column.append(&title);
    column.append(&value);
    (column, title, value)
}
fn surface(
    info: &SessionInfo,
    monitor: &gtk::gdk::Monitor,
    layer: Layer,
    footer: bool,
) -> gtk::Window {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(layer);
    window.set_namespace(Some("keek-footer"));
    // Slot margins include reserved space, so place surfaces against the full monitor.
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_monitor(Some(monitor));
    let rect = info.placement;
    window.set_margin(Edge::Left, rect.x);
    window.set_margin(
        Edge::Top,
        if footer {
            rect.y + rect.height - crate::slots::FOOTER_OVERLAP
        } else {
            rect.y
        },
    );
    window.set_default_size(
        rect.width,
        if footer {
            crate::slots::FOOTER_HEIGHT
        } else {
            rect.height
        },
    );
    window.set_resizable(false);
    window.add_css_class(if footer { "footer" } else { "hover-surface" });
    window.add_css_class(footer_view::SessionColour::for_slot(info.slot).css_class());
    window
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
    let footer = surface(&info, &monitor, Layer::Top, true);
    let surface = surface(&info, &monitor, Layer::Bottom, false);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    row.set_margin_start(16);
    row.set_margin_end(16);
    row.set_valign(gtk::Align::Center);
    let identity = gtk::Box::new(gtk::Orientation::Vertical, 4);
    identity.set_hexpand(true);
    identity.set_size_request(100, -1);
    let view = footer_view::for_session(
        &info,
        &snapshot,
        chrono::Utc::now(),
        footer_view::Hover::Away,
        None,
    );
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    heading.append(&label("➤", "cursor"));
    heading.append(&label(&view.project, "value"));
    heading.append(&label(&view.branch, "dim"));
    identity.append(&heading);
    identity.append(&label(&view.goal, "dim"));
    row.append(&identity);
    let (status, state_label, detail) = column("", "", 100);
    status.set_hexpand(true);
    row.append(&status);
    let checked = gtk::Box::new(gtk::Orientation::Vertical, 4);
    checked.set_size_request(190, -1);
    row.append(&checked);
    let (clock, elapsed_label, elapsed) = column("Elapsed", "0:00", 96);
    row.append(&clock);
    let button = gtk::Button::with_label("Take control");
    button.set_valign(gtk::Align::Center);
    button.set_size_request(-1, 40);
    row.append(&button);
    footer.set_child(Some(&row));
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(state);
    let slot = info.slot;
    motion.connect_enter(move |_, _, _| {
        if let Some(state) = weak.upgrade()
            && let Ok(mut data) = state.try_borrow_mut()
            && let Some(card) = data.cards.get_mut(&slot)
        {
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
    let card = Card {
        info,
        snapshot,
        frozen,
        hover: footer_view::Hover::Away,
        footer,
        surface,
        state_label,
        detail,
        checked,
        elapsed_label,
        elapsed,
        button,
        watcher: None,
    };
    card.draw();
    Some(card)
}
fn style() {
    let t = theme::load();
    let provider = gtk::CssProvider::new();
    provider.load_from_data(&format!("window.footer {{ background: {}; color: {}; font-family: 'JetBrains Mono'; font-size: 12px; border: 2px solid {}; border-top: 2px solid {}; border-radius: 0 0 8px 8px; }} window.violet {{border-top-color:{};}} .violet .cursor {{color:{};}} .cyan .cursor {{color:{};}} .dim {{color:{};}} .hint {{color:{};}} .value {{font-weight:700; font-feature-settings: 'tnum';}} button {{background:alpha({},0.10);color:{};border:1px solid {};border-radius:8px;font-weight:700;padding:0 18px;}} button:hover,button.hinted {{background:alpha({},0.22);}} window.hover-surface {{background:transparent;}} .tick {{min-width:10px;min-height:10px;border-radius:2px;}} .passed {{background:{};}} .failed {{background:{};}} .skipped {{border:1px solid {};color:{};}} .unmarked {{border:1px solid {};}}",t.background,t.foreground,t.border,footer_view::SessionColour::Cyan.color(),footer_view::SessionColour::Violet.color(),footer_view::SessionColour::Violet.color(),footer_view::SessionColour::Cyan.color(),t.dim,t.accent,t.accent,t.accent,t.accent,t.accent,t.green,t.red,t.yellow,t.yellow,t.dim));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
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
    style();
    let main_loop = glib::MainLoop::new(None, false);
    let state = Rc::new(RefCell::new(State {
        cards: HashMap::new(),
        visible: HashSet::new(),
        tick: None,
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
                ViewerMessage::Opened(info, mut snapshots) => {
                    let slot = info.slot;
                    let snapshot = snapshots.borrow_and_update().clone();
                    if let Some(old) = message_state.borrow_mut().cards.remove(&slot) {
                        old.close();
                    }
                    let Some(card) = create(info, snapshot, &message_state) else {
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
                                card.snapshot = snapshot;
                                card.draw();
                            }
                        }
                    });
                    if let Some(card) = message_state.borrow_mut().cards.get_mut(&slot) {
                        card.watcher = Some(watcher);
                    }
                    State::visibility(&message_state);
                }
                ViewerMessage::Closed(slot) => {
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
}
