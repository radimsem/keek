use super::{
    end_card,
    footer_content::FooterContent,
    footer_view::{self, FooterButton, FooterView, Hover, SessionColour},
    paused_card,
    ui::{primary_click, surface},
};
use crate::{ipc::SessionSnapshot, slots::CardGeometry};
use gtk4::{self as gtk, prelude::*};
use gtk4_layer_shell::{Layer, LayerShell};

pub(super) struct FocusCard {
    pub hover: Hover,
    footer: gtk::Window,
    surface: gtk::Window,
    paused: gtk::Window,
    body: gtk::Overlay,
    content: FooterContent,
    end_card: Option<end_card::EndCard>,
}

impl FocusCard {
    pub fn new(
        monitor: &gtk::gdk::Monitor,
        geometry: CardGeometry,
        view: &FooterView,
        snapshot: &SessionSnapshot,
        activate: impl Fn(FooterButton) + 'static,
        activate_body: impl Fn() + Clone + 'static,
        hover: impl Fn(Hover) + 'static,
    ) -> Self {
        let footer = surface(monitor, Layer::Top, "keek-footer", geometry.footer);
        let surface = surface(monitor, Layer::Bottom, "keek-footer", geometry.body);
        let paused = paused_card::new(geometry, monitor);
        let body = gtk::Overlay::new();
        let edge = gtk::Box::new(gtk::Orientation::Vertical, 0);
        edge.add_css_class("card-edge");
        edge.set_can_target(false);
        body.add_overlay(&edge);
        surface.set_child(Some(&body));
        let content = FooterContent::new(view, activate);
        footer.set_child(Some(&content.row));
        surface.add_controller(primary_click(activate_body.clone()));
        paused.add_controller(primary_click(activate_body));

        let motion = gtk::EventControllerMotion::new();
        let hover = std::rc::Rc::new(hover);
        let enter = hover.clone();
        motion.connect_enter(move |_, _, _| enter(Hover::Over));
        motion.connect_leave(move |_| hover(Hover::Away));
        surface.add_controller(motion);

        let mut card = Self {
            hover: Hover::Away,
            footer,
            surface,
            paused,
            body,
            content,
            end_card: None,
        };
        card.update_end(snapshot, geometry);
        card
    }

    pub fn update_end(&mut self, snapshot: &SessionSnapshot, geometry: CardGeometry) {
        end_card::update_slot(&mut self.end_card, &self.body, snapshot, geometry);
        if self.end_card.is_some() {
            self.footer.set_layer(Layer::Bottom);
        }
    }

    pub fn draw(&self, view: &FooterView, colour: SessionColour) {
        let mut classes = vec!["footer", colour.css_class()];
        if let Some(class) = view.label.css_class() {
            classes.push(class);
        }
        self.footer.set_css_classes(&classes);
        classes[0] = "hover-surface";
        self.surface.set_css_classes(&classes);
        self.content.draw(view);
    }

    pub fn set_visible(&self, shown: bool, snapshot: &SessionSnapshot) {
        let paused = shown && footer_view::is_paused(snapshot);
        if paused && !self.paused.is_visible() {
            self.footer.set_visible(false);
        }
        // Mapping the footer last keeps its overlapping edge above the card surfaces.
        self.surface.set_visible(shown);
        self.paused.set_visible(paused);
        self.footer.set_visible(shown);
    }

    pub fn close(self) {
        self.footer.close();
        self.surface.close();
        self.paused.close();
    }
}
