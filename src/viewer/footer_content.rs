use super::{
    footer_actions::FooterActions,
    footer_view::{FooterButton, FooterView, StateLabel, Tick},
};
use gtk4::{self as gtk, prelude::*};

pub(super) struct FooterContent {
    pub row: gtk::Box,
    state_label: gtk::Label,
    detail: gtk::Label,
    checked: gtk::Box,
    elapsed_label: gtk::Label,
    elapsed: gtk::Label,
    actions: FooterActions,
}

impl FooterContent {
    pub fn new(view: &FooterView, activate: impl Fn(FooterButton) + 'static) -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        row.set_margin_start(16);
        row.set_margin_end(16);
        row.set_valign(gtk::Align::Center);
        let identity = gtk::Box::new(gtk::Orientation::Vertical, 4);
        identity.set_size_request(100, -1);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        heading.append(&label("➤", "cursor"));
        heading.append(&label(&view.project, "value"));
        heading.append(&label(&view.branch, "dim"));
        identity.append(&heading);
        identity.append(&label(&view.goal, "dim"));
        let (status, state_label, detail) = column("", "", 100);
        // Equal halves keep the state column's left edge still while its text changes length.
        let flexible = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        flexible.set_homogeneous(true);
        flexible.set_hexpand(true);
        flexible.append(&identity);
        flexible.append(&status);
        row.append(&flexible);
        let checked = gtk::Box::new(gtk::Orientation::Vertical, 4);
        checked.set_size_request(190, -1);
        row.append(&checked);
        let (clock, elapsed_label, elapsed) = column("Elapsed", "0:00", 96);
        row.append(&clock);
        let actions = FooterActions::new(activate);
        row.append(&actions.container);
        Self {
            row,
            state_label,
            detail,
            checked,
            elapsed_label,
            elapsed,
            actions,
        }
    }

    pub fn draw(&self, view: &FooterView) {
        self.actions.draw(view);
        self.state_label.set_text(view.label.text());

        self.detail.set_text(&view.detail);
        self.elapsed_label.set_text(view.elapsed_label);
        self.elapsed.set_text(&view.elapsed);
        self.state_label
            .set_css_classes(&[if view.label == StateLabel::ViewOnly {
                "hint"
            } else {
                view.label.css_class().unwrap_or("dim")
            }]);
        self.state_label.add_css_class("state");
        while let Some(child) = self.checked.first_child() {
            self.checked.remove(&child);
        }
        let Some(checked) = &view.checked else {
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
        for &kind in &checked.ticks {
            // An empty Box is exactly the CSS 10px square; a Label would be as tall as its line.
            let tick = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            tick.add_css_class("tick");
            tick.add_css_class(kind.css_class());
            tick.set_valign(gtk::Align::Center);
            let mark = match kind {
                Tick::Skipped => Some(label("−", "skipped")),
                Tick::InProgress => Some(label("", "check-ring")),
                Tick::Passed | Tick::Failed | Tick::Unmarked => None,
            };
            let Some(mark) = mark else {
                value.append(&tick);
                continue;
            };
            // The mark is drawn over the square without changing its size.
            let cell = gtk::Overlay::new();
            cell.set_child(Some(&tick));
            cell.set_valign(gtk::Align::Center);
            mark.set_halign(gtk::Align::Center);
            mark.set_valign(gtk::Align::Center);
            mark.set_can_target(false);
            cell.add_overlay(&mark);
            cell.set_measure_overlay(&mark, false);
            value.append(&cell);
        }
        if checked.overflow > 0 {
            value.append(&label(&format!("+{}", checked.overflow), "dim"));
        }
        self.checked.append(&title);
        self.checked.append(&value);
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
