use super::{
    end_card,
    footer_actions::keycaps,
    footer_content::{checked_row, label},
    footer_view::SessionColour,
    rail_panels,
    rail_view::{HoverSpot, RailCardView, RailHover},
    ui::{primary_click, surface},
};
use crate::{
    ipc::SessionState,
    slots::{RAIL_CARD_CHROME, Slot},
};
use gtk4::{self as gtk, prelude::*};
use gtk4_layer_shell::Layer;
use std::path::PathBuf;

pub(super) struct RailCard {
    pub window: gtk::Window,
    pub over: bool,
    pub locked: bool,
    parts: RailCardParts,
    panel: Option<HoverPanel>,
    frame_loader: Option<(PathBuf, gtk::glib::JoinHandle<()>)>,
}

struct HoverPanel {
    spot: HoverSpot,
    view: RailHover,
    window: gtk::Window,
}

impl RailCard {
    pub fn new(
        monitor: &gtk::gdk::Monitor,
        rect: Slot,
        colour: SessionColour,
        view: &RailCardView,
        show: impl Fn() + 'static,
        hover: impl Fn(bool) + 'static,
    ) -> Self {
        let window = surface(monitor, Layer::Top, "keek-rail", rect);
        window.add_css_class("rail-surface");
        let (widget, parts) = rail_card_widget(view, colour);
        parts
            .still
            .set_height_request((rect.height - RAIL_CARD_CHROME).max(0));
        window.set_child(Some(&widget));
        window.add_controller(primary_click(show));
        let motion = gtk::EventControllerMotion::new();
        let hover = std::rc::Rc::new(hover);
        let enter = hover.clone();
        motion.connect_enter(move |_, _, _| enter(true));
        motion.connect_leave(move |_| hover(false));
        window.add_controller(motion);
        Self {
            window,
            over: false,
            locked: view.locked,
            parts,
            panel: None,
            frame_loader: None,
        }
    }

    pub fn set_texture(&self, texture: &gtk::gdk::Texture) {
        self.parts.picture.set_paintable(Some(texture));
    }

    pub fn load_end_frame(&mut self, state: &SessionState) {
        let frame = match state {
            SessionState::Rendering { frame, .. } => frame,
            SessionState::Ended(end) => &end.frame,
            SessionState::Opening | SessionState::Open | SessionState::Closing => return,
        };
        if let Some(path) = frame
            && self.parts.picture.paintable().is_none()
            && self
                .frame_loader
                .as_ref()
                .is_none_or(|(loading, _)| loading != path)
        {
            if let Some((_, loader)) = self.frame_loader.take() {
                loader.abort();
            }
            let loader = end_card::load_frame(&self.parts.picture, path.clone());
            self.frame_loader = Some((path.clone(), loader));
        }
    }

    pub fn draw(&self, view: &RailCardView) {
        self.parts.draw(view);
    }

    pub fn sync_hover(
        &mut self,
        monitor: &gtk::gdk::Monitor,
        placement: Option<HoverSpot>,
        view: RailHover,
    ) {
        let placement = placement.filter(|_| self.over && !self.locked);
        set_class(&self.parts.root, "hovered", placement.is_some());
        let Some(spot) = placement else {
            self.close_panel();
            return;
        };
        if let Some(panel) = &mut self.panel
            && panel.spot == spot
        {
            if panel.view != view {
                panel
                    .window
                    .set_child(Some(&rail_panels::hover_widget(&view)));
                panel.view = view;
            }
            return;
        }
        self.close_panel();
        let window = rail_panels::hover_surface(monitor, spot, &view);
        window.set_visible(true);
        self.panel = Some(HoverPanel { spot, view, window });
    }

    fn close_panel(&mut self) {
        if let Some(panel) = self.panel.take() {
            panel.window.close();
        }
    }

    pub fn close(mut self) {
        if let Some((_, loader)) = self.frame_loader.take() {
            loader.abort();
        }
        self.close_panel();
        self.window.close();
    }
}

pub(super) struct RailCardParts {
    root: gtk::Box,
    still: gtk::Overlay,
    picture: gtk::Picture,
    age: gtk::Label,
    elapsed: gtk::Label,
    needs_you: gtk::Label,
    line: gtk::Label,
    checked: gtk::Box,
    keys: gtk::Box,
}

impl RailCardParts {
    fn draw(&self, view: &RailCardView) {
        set_class(&self.root, "locked", view.locked);
        set_class(&self.root, "needs-you", view.needs_you.is_some());
        self.elapsed.set_text(&view.elapsed);
        self.age.set_text(view.still.as_deref().unwrap_or(""));
        self.age.set_visible(view.still.is_some());
        self.needs_you.set_visible(view.needs_you.is_some());
        self.line
            .set_text(view.needs_you.as_deref().unwrap_or(&view.line));
        self.line.set_css_classes(&[if view.needs_you.is_some() {
            "rail-reason"
        } else {
            view.verdict
                .and_then(|verdict| verdict.css_class())
                .unwrap_or("dim")
        }]);
        while let Some(child) = self.checked.first_child() {
            self.checked.remove(&child);
        }
        if let Some(checked) = &view.checked {
            let row = checked_row(checked);
            row.update_property(&[gtk::accessible::Property::Label(&checked.accessible)]);
            self.checked.append(&row);
        }
        while let Some(child) = self.keys.first_child() {
            self.keys.remove(&child);
        }
        // A hidden child takes no Box spacing, so the row ends at the ticks.
        self.keys.set_visible(view.keys.is_some());
        if let Some(keys) = &view.keys {
            self.keys.append(&keycaps(keys.iter().map(String::as_str)));
        }
    }
}

pub(super) fn rail_card_widget(
    view: &RailCardView,
    colour: SessionColour,
) -> (gtk::Widget, RailCardParts) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("rail-card");
    root.add_css_class(colour.css_class());
    root.set_overflow(gtk::Overflow::Hidden);
    root.set_accessible_role(gtk::AccessibleRole::Button);
    root.update_property(&[gtk::accessible::Property::Label(&view.accessible)]);
    let still = gtk::Overlay::new();
    still.add_css_class("rail-still");
    still.set_height_request(256);
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Cover);
    still.add_overlay(&picture);
    still.set_measure_overlay(&picture, false);
    let age = label("", "rail-age");
    age.set_halign(gtk::Align::Start);
    age.set_valign(gtk::Align::End);
    age.set_margin_start(8);
    age.set_margin_bottom(8);
    still.add_overlay(&age);
    still.set_measure_overlay(&age, false);
    root.append(&still);
    let edge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    edge.add_css_class("rail-session-edge");
    root.append(&edge);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.add_css_class("rail-text");
    text.set_vexpand(true);
    text.set_homogeneous(true);
    let identity = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    identity.append(&label("➤", "cursor"));
    identity.append(&label(&view.project, "rail-project"));
    let branch = label(&view.branch, "dim");
    branch.set_hexpand(true);
    identity.append(&branch);
    let elapsed = label("", "value");
    identity.append(&elapsed);
    text.append(&identity);
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let needs_you = label("Needs you", "rail-needs-you");
    status.append(&needs_you);
    let line = label("", "dim");
    status.append(&line);
    text.append(&status);
    let summary = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let checked = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    checked.set_hexpand(true);
    summary.append(&checked);
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    keys.add_css_class("dim");
    keys.set_valign(gtk::Align::Center);
    summary.append(&keys);
    text.append(&summary);
    root.append(&text);
    let parts = RailCardParts {
        root: root.clone(),
        still,
        picture,
        age,
        elapsed,
        needs_you,
        line,
        checked,
        keys,
    };
    parts.draw(view);
    (root.upcast(), parts)
}

fn set_class(widget: &gtk::Box, class: &str, enabled: bool) {
    if enabled {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}
