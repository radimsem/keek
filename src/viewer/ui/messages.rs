use super::{State, create};
use crate::viewer::ViewerMessage;
use crate::viewer::rail_view::ShowKeys;
use gtk4::{glib, prelude::*};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};
use tokio::sync::mpsc;

pub fn run(mut receiver: mpsc::Receiver<ViewerMessage>, handle: tokio::runtime::Handle) {
    crate::viewer::style::install_style();
    let main_loop = glib::MainLoop::new(None, false);
    let state = Rc::new(RefCell::new(State {
        cards: HashMap::new(),
        missing: HashMap::new(),
        visible: HashSet::new(),
        tick: None,
        notes: HashMap::new(),
        commands: None,
        show_keys: ShowKeys::default(),
    }));
    let (workspace_tx, mut workspace_rx) = mpsc::channel(8);
    let (show_tx, mut show_rx) = mpsc::channel(8);
    let (keys_tx, mut keys_rx) = mpsc::channel(8);
    let monitor_task = handle.spawn(crate::viewer::events::watch(workspace_tx, show_tx, keys_tx));
    let keys_state = state.clone();
    let keys_watcher = glib::spawn_future_local(async move {
        while let Some(keys) = keys_rx.recv().await {
            {
                let mut data = keys_state.borrow_mut();
                if data.show_keys == keys {
                    continue;
                }
                data.show_keys = keys;
                for card in data.cards.values_mut() {
                    card.show_keys = keys;
                }
            }
            State::visibility(&keys_state);
        }
    });
    let show_state = state.clone();
    let show_watcher = glib::spawn_future_local(async move {
        while let Some(target) = show_rx.recv().await {
            show_state.borrow().show(target);
        }
    });
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
                    let info = *info;
                    let slot = info.slot;
                    let name = info.name.clone();
                    let snapshot = snapshots.borrow_and_update().clone();
                    if let Some(old) = message_state.borrow_mut().missing.remove(&slot) {
                        old.abort();
                    }
                    if let Some(old) = message_state.borrow_mut().cards.remove(&slot) {
                        old.close();
                    }
                    let show_keys = message_state.borrow().show_keys;
                    let Some(card) = create(info, snapshot, show_keys, &message_state) else {
                        let commands = message_state.borrow().commands.clone();
                        if let Some(commands) = commands {
                            let task =
                                crate::viewer::missing_card::watch(snapshots, name, commands);
                            message_state.borrow_mut().missing.insert(slot, task);
                        }
                        State::visibility(&message_state);
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
                                card.update(snapshot);
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
                ViewerMessage::Arranged(cards) => {
                    {
                        let mut data = message_state.borrow_mut();
                        for (slot, spot) in cards {
                            if data.cards.get(&slot).is_some_and(|card| card.spot != spot)
                                && let Some(card) = data.cards.remove(&slot)
                            {
                                data.cards
                                    .insert(slot, card.rearranged(spot, &message_state));
                            }
                        }
                    }
                    State::visibility(&message_state);
                }
                ViewerMessage::Still(slot, still) => {
                    if let Some(card) = message_state.borrow_mut().cards.get_mut(&slot) {
                        card.set_still(still);
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
    show_watcher.abort();
    keys_watcher.abort();
    monitor_task.abort();
    let mut state = state.borrow_mut();
    if let Some(tick) = state.tick.take() {
        tick.remove();
    }
    for (_, card) in state.cards.drain() {
        card.close();
    }
    for (_, (_, note)) in state.notes.drain() {
        note.close();
    }
    for (_, task) in state.missing.drain() {
        task.abort();
    }
}
