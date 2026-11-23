use super::footer_view::{FooterButton, FooterView, Shortcut, StateLabel};
use gtk4::{self as gtk, prelude::*};
use std::{cell::RefCell, rc::Rc};

pub(super) struct FooterActions {
    pub container: gtk::CenterBox,
    row: gtk::Box,
    shown: RefCell<Vec<(FooterButton, Option<Shortcut>)>>,
    activate: Rc<dyn Fn(FooterButton)>,
}

impl FooterActions {
    pub fn new(activate: impl Fn(FooterButton) + 'static) -> Self {
        let container = gtk::CenterBox::new();
        container.set_size_request(400, -1);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("footer-actions");
        container.set_end_widget(Some(&row));
        Self {
            container,
            row,
            shown: RefCell::new(Vec::new()),
            activate: Rc::new(activate),
        }
    }

    pub fn draw(&self, view: &FooterView) {
        if view.label == StateLabel::ViewOnly {
            self.row.add_css_class("hinted");
        } else {
            self.row.remove_css_class("hinted");
        }
        let actions = view.buttons.iter().map(|&kind| {
            let shortcut = view
                .shortcuts
                .iter()
                .find_map(|&(button, shortcut)| (button == kind).then_some(shortcut));
            (kind, shortcut)
        });
        let mut shown = self.shown.borrow_mut();
        if shown.iter().copied().eq(actions.clone()) {
            return;
        }
        while let Some(child) = self.row.first_child() {
            self.row.remove(&child);
        }
        shown.clear();
        for (kind, shortcut) in actions {
            if let Some(shortcut) = shortcut {
                self.row.append(&keycaps(shortcut.keys()));
            }
            let button = gtk::Button::with_label(if kind == FooterButton::Dismiss {
                "×"
            } else {
                kind.text()
            });
            button.set_size_request(
                if kind == FooterButton::Dismiss {
                    40
                } else {
                    -1
                },
                40,
            );
            match kind {
                FooterButton::TakeControl => button.add_css_class("take-control"),
                FooterButton::Done => button.add_css_class("done"),
                FooterButton::StepAway | FooterButton::OpenVideo | FooterButton::OpenFolder => {
                    button.add_css_class("secondary");
                }
                FooterButton::Dismiss => button.add_css_class("dismiss"),
                FooterButton::Resume => {}
            }
            button.update_property(&[gtk::accessible::Property::Label(kind.text())]);
            let activate = self.activate.clone();
            button.connect_clicked(move |_| activate(kind));
            self.row.append(&button);
            shown.push((kind, shortcut));
        }
    }
}

pub(super) fn keycaps<'a>(keys: impl IntoIterator<Item = &'a str>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    for key in keys {
        let keycap = gtk::Label::new(Some(key));
        keycap.add_css_class("keycap");
        row.append(&keycap);
    }
    row
}
