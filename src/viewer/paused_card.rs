use super::{footer_view::StateLabel, ui};
use crate::slots::CardGeometry;
use gtk4::{self as gtk, prelude::*};
use gtk4_layer_shell::Layer;

const PANEL_MARGIN: i32 = 16;

#[cfg(test)]
pub(super) fn panel_width(geometry: CardGeometry) -> i32 {
    (geometry.window.width - 2 * PANEL_MARGIN).max(1)
}

pub(super) fn new(geometry: CardGeometry, monitor: &gtk::gdk::Monitor) -> gtk::Window {
    let window = ui::surface(monitor, Layer::Top, "keek-paused", geometry.window);
    window.add_css_class("paused-surface");

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
    panel.add_css_class("paused-panel");
    panel.set_halign(gtk::Align::Center);
    panel.set_valign(gtk::Align::Center);
    panel.set_margin_start(PANEL_MARGIN);
    panel.set_margin_end(PANEL_MARGIN);
    let title = gtk::Label::new(Some(StateLabel::TakeoverPaused.text()));
    title.add_css_class("paused-title");
    title.set_justify(gtk::Justification::Center);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    let detail = gtk::Label::new(Some(
        "Click the session to carry on. The Verifier waits until you press Done.",
    ));
    detail.set_justify(gtk::Justification::Center);
    detail.set_wrap(true);
    detail.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    panel.append(&title);
    panel.append(&detail);
    window.set_child(Some(&panel));
    window
}
