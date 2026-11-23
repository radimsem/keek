use super::{
    rail_view::{HOVER_WIDTH, HoverMark, HoverSpot, LOCKED_NOTE, NoteSpot, RailHover},
    ui::surface,
};
use crate::slots::Slot;
use gtk4::{self as gtk, prelude::*};
use gtk4_layer_shell::{Edge, Layer, LayerShell};

pub(super) fn hover_surface(
    monitor: &gtk::gdk::Monitor,
    spot: HoverSpot,
    view: &RailHover,
) -> gtk::Window {
    let (x, y) = match spot {
        HoverSpot::LeftOf { x, top } => (x, top),
        HoverSpot::Above { x, bottom: _ } => (x, 0),
    };
    let window = panel_surface(monitor, x, y, HOVER_WIDTH, &hover_widget(view));
    if let HoverSpot::Above { bottom, .. } = spot {
        window.set_anchor(Edge::Top, false);
        window.set_anchor(Edge::Bottom, true);
        window.set_margin(Edge::Bottom, monitor.geometry().height() - bottom);
    }
    window
}

pub(super) fn locked_surface(monitor: &gtk::gdk::Monitor, spot: NoteSpot) -> gtk::Window {
    let text = wrapping_label(LOCKED_NOTE);
    text.add_css_class("rail-locked-note");
    panel_surface(monitor, spot.x, spot.top, spot.width, &text)
}

fn panel_surface(
    monitor: &gtk::gdk::Monitor,
    x: i32,
    y: i32,
    width: i32,
    child: &impl IsA<gtk::Widget>,
) -> gtk::Window {
    let window = surface(
        monitor,
        Layer::Overlay,
        "keek-rail",
        Slot {
            x,
            y,
            width,
            height: -1,
        },
    );
    window.add_css_class("rail-surface");
    child.set_can_target(false);
    window.set_child(Some(child));
    window.connect_realize(|window| {
        if let Some(surface) = window.surface() {
            surface.set_input_region(Some(&gtk::cairo::Region::create()));
        }
    });
    window
}

pub(super) fn hover_widget(hover: &RailHover) -> gtk::Widget {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.set_width_request(HOVER_WIDTH);
    panel.add_css_class("rail-hover");
    panel.set_can_target(false);
    panel.append(&wrapping_label(&hover.goal));
    if !hover.checks.is_empty() {
        let checks = gtk::Box::new(gtk::Orientation::Vertical, 9);
        for check in &hover.checks {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let text = wrapping_label(&check.text);
            text.set_hexpand(true);
            let class = match check.mark {
                HoverMark::Passed => "passed",
                HoverMark::Failed => "failed",
                HoverMark::Skipped => "skipped",
                HoverMark::Unmarked => "unmarked",
                HoverMark::Checking => "checking",
            };
            let mark = gtk::Label::new(Some(if check.mark == HoverMark::Passed {
                "✓"
            } else {
                ""
            }));
            mark.add_css_class("rail-check-mark");
            mark.add_css_class(class);
            mark.set_valign(gtk::Align::Start);
            row.append(&mark);
            row.append(&text);
            if check.mark == HoverMark::Unmarked {
                text.add_css_class("dim");
            }
            if check.mark == HoverMark::Checking {
                text.add_css_class("value");
                let pill = gtk::Label::new(Some("checking"));
                pill.add_css_class("rail-checking-pill");
                pill.set_valign(gtk::Align::Start);
                row.append(&pill);
            }
            checks.append(&row);
        }
        panel.append(&checks);
    }
    let hint = wrapping_label(hover.hint);
    hint.add_css_class("dim");
    panel.append(&hint);
    panel.upcast()
}

fn wrapping_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label
}
