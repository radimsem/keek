use super::{
    footer_view::{FooterButton, FooterView, Tick},
    theme::Theme,
    verdict_view::{self, Icon},
};
use crate::ipc::{SessionSnapshot, SessionState};
use gtk4::{self as gtk, prelude::*};

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class(class);
    label
}

pub struct EndCard {
    pub overlay: gtk::Overlay,
    panel: gtk::Box,
    progress: Option<gtk::ProgressBar>,
    tally: gtk::Label,
    width: i32,
    frame_loader: Option<gtk::glib::JoinHandle<()>>,
}
impl EndCard {
    pub fn new(snapshot: &SessionSnapshot, geometry: crate::slots::CardGeometry) -> Option<Self> {
        let frame = match &snapshot.state {
            SessionState::Rendering { frame, .. } => frame,
            SessionState::Ended(end) => &end.frame,
            SessionState::Opening | SessionState::Open | SessionState::Closing => return None,
        };
        let overlay = gtk::Overlay::new();
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_size_request(geometry.window.width, geometry.window.height);
        let frame_loader = frame.clone().map(|path| {
            let weak = picture.downgrade();
            gtk::glib::spawn_future_local(async move {
                let decoded = gtk::gio::spawn_blocking(move || {
                    let image = gtk::gdk_pixbuf::Pixbuf::from_file(path).ok()?;
                    Some((
                        image.width(),
                        image.height(),
                        image.has_alpha(),
                        image.rowstride() as usize,
                        image.read_pixel_bytes().as_ref().to_vec(),
                    ))
                })
                .await;
                if let (Ok(Some((width, height, alpha, stride, pixels))), Some(picture)) =
                    (decoded, weak.upgrade())
                {
                    let format = if alpha {
                        gtk::gdk::MemoryFormat::R8g8b8a8
                    } else {
                        gtk::gdk::MemoryFormat::R8g8b8
                    };
                    let texture = gtk::gdk::MemoryTexture::new(
                        width,
                        height,
                        format,
                        &gtk::glib::Bytes::from_owned(pixels),
                        stride,
                    );
                    picture.set_paintable(Some(&texture));
                }
            })
        });
        let frame = gtk::Fixed::new();
        frame.put(
            &picture,
            f64::from(geometry.window.x - geometry.body.x),
            f64::from(geometry.window.y - geometry.body.y),
        );
        overlay.set_child(Some(&frame));
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("end-scrim");
        scrim.set_margin_start(geometry.window.x - geometry.body.x);
        scrim.set_margin_top(geometry.window.y - geometry.body.y);
        scrim.set_margin_end(
            geometry.body.x + geometry.body.width - geometry.window.x - geometry.window.width,
        );
        scrim.set_margin_bottom(
            geometry.body.y + geometry.body.height - geometry.window.y - geometry.window.height,
        );
        overlay.add_overlay(&scrim);
        let view = verdict_view::for_snapshot(snapshot)?;
        let (panel, progress, tally) = panel(view, geometry.body.width);
        overlay.add_overlay(&panel);
        Some(Self {
            overlay,
            panel,
            progress,
            tally,
            width: geometry.body.width,
            frame_loader,
        })
    }
    pub fn update(&mut self, snapshot: &SessionSnapshot) {
        let Some(view) = verdict_view::for_snapshot(snapshot) else {
            return;
        };
        match view.mode {
            verdict_view::PanelMode::Rendering(percent) => {
                if let Some(progress) = &self.progress {
                    progress.set_fraction(f64::from(percent.0) / 100.0);
                    self.tally.set_text(&view.tally);
                }
            }
            verdict_view::PanelMode::Ended => {
                if self.progress.is_some() {
                    self.overlay.remove_overlay(&self.panel);
                    let (panel, progress, tally) = panel(view, self.width);
                    self.overlay.add_overlay(&panel);
                    self.panel = panel;
                    self.progress = progress;
                    self.tally = tally;
                }
            }
        }
    }
}
impl Drop for EndCard {
    fn drop(&mut self) {
        if let Some(loader) = self.frame_loader.take() {
            loader.abort();
        }
    }
}
pub fn update_slot(
    card: &mut Option<EndCard>,
    body: &gtk::Overlay,
    snapshot: &SessionSnapshot,
    geometry: crate::slots::CardGeometry,
) {
    match card {
        Some(end) => end.update(snapshot),
        None => {
            *card = EndCard::new(snapshot, geometry);
            body.set_child(card.as_ref().map(|end| &end.overlay));
        }
    }
}
fn panel(
    view: verdict_view::PanelView,
    width: i32,
) -> (gtk::Box, Option<gtk::ProgressBar>, gtk::Label) {
    let theme = super::theme::load();
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 20);
    panel.add_css_class("verdict-panel");
    panel.set_halign(gtk::Align::Center);
    panel.set_margin_start(16);
    panel.set_margin_end(16);
    panel.set_valign(gtk::Align::Center);
    panel.set_size_request(
        if matches!(view.mode, verdict_view::PanelMode::Rendering(_)) {
            560
        } else {
            720
        }
        .min((width - 32).max(1)),
        -1,
    );
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let words = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let title = label(
        &view.title,
        if matches!(view.mode, verdict_view::PanelMode::Rendering(_)) {
            "panel-title"
        } else {
            "verdict-title"
        },
    );
    if matches!(view.mode, verdict_view::PanelMode::Ended) {
        title.add_css_class(view.tone.css_class());
    }
    words.append(&title);
    if !view.subtitle.is_empty() {
        let subtitle = label(&view.subtitle, "panel-meta");
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
        words.append(&subtitle);
    }
    words.set_hexpand(true);
    if matches!(view.mode, verdict_view::PanelMode::Ended) {
        header.append(&icon(view.icon, &theme));
    }
    header.append(&words);
    let tally = label(
        &view.tally,
        if matches!(view.mode, verdict_view::PanelMode::Rendering(_)) {
            "panel-title"
        } else {
            "panel-meta"
        },
    );
    tally.set_valign(gtk::Align::End);
    header.append(&tally);
    panel.append(&header);
    let progress = if let verdict_view::PanelMode::Rendering(percent) = view.mode {
        let progress = gtk::ProgressBar::new();
        progress.set_fraction(f64::from(percent.0) / 100.0);
        panel.append(&progress);
        Some(progress)
    } else {
        panel.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let checks = gtk::Box::new(gtk::Orientation::Vertical, 12);
        for row in view.rows {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            line.append(&check_icon(row.tick, &theme));
            let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let text = label(&row.text, "check-text");
            text.set_wrap(true);
            text.set_hexpand(true);
            heading.append(&text);
            if row.added {
                heading.append(&label("added", "added-tag"));
            }
            body.append(&heading);
            if let Some(comment) = row.comment {
                let comment = label(&comment, "check-comment");
                comment.set_wrap(true);
                body.append(&comment);
            }
            line.append(&body);
            checks.append(&line);
        }
        panel.append(&checks);
        None
    };
    (panel, progress, tally)
}

fn paint(color: &str, context: &gtk::cairo::Context) {
    if let Ok(color) = gtk::gdk::RGBA::parse(color) {
        context.set_source_rgba(
            color.red().into(),
            color.green().into(),
            color.blue().into(),
            1.0,
        );
    }
}
fn icon(kind: Icon, theme: &Theme) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(32);
    area.set_content_height(32);
    area.set_valign(gtk::Align::Start);
    let theme = theme.clone();
    area.set_draw_func(move |_, c, _, _| {
        paint(
            match kind {
                Icon::Pass => &theme.green,
                Icon::NeedsFixes => &theme.red,
                Icon::PartlyVerified => &theme.yellow,
                Icon::Dim => &theme.dim,
            },
            c,
        );
        c.set_line_width(2.0);
        if kind == Icon::Dim {
            c.set_dash(&[3.0, 3.0], 0.0);
        }
        c.arc(16.0, 16.0, 13.0, 0.0, std::f64::consts::TAU);
        let _ = c.stroke();
        c.set_dash(&[], 0.0);
        match kind {
            Icon::Pass => {
                c.move_to(8.0, 16.0);
                c.line_to(14.0, 22.0);
                c.line_to(24.0, 10.0);
                let _ = c.stroke();
            }
            Icon::NeedsFixes => {
                c.move_to(16.0, 8.0);
                c.line_to(16.0, 19.0);
                let _ = c.stroke();
                c.arc(16.0, 24.0, 1.0, 0.0, std::f64::consts::TAU);
                let _ = c.fill();
            }
            Icon::PartlyVerified => {
                c.arc(
                    16.0,
                    16.0,
                    13.0,
                    -std::f64::consts::FRAC_PI_2,
                    std::f64::consts::FRAC_PI_2,
                );
                c.close_path();
                let _ = c.fill();
            }
            Icon::Dim => {
                for x in [12.0, 20.0] {
                    c.move_to(x, 10.0);
                    c.line_to(x, 22.0);
                }
                let _ = c.stroke();
            }
        }
    });
    area
}
fn check_icon(tick: Tick, theme: &Theme) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(16);
    area.set_content_height(20);
    area.set_valign(gtk::Align::Start);
    let theme = theme.clone();
    area.set_draw_func(move |_, c, _, _| {
        paint(
            match tick {
                Tick::Passed => &theme.green,
                Tick::Failed => &theme.red,
                Tick::Skipped => &theme.yellow,
                Tick::Unmarked | Tick::InProgress => &theme.dim,
            },
            c,
        );
        c.set_line_width(2.0);
        match tick {
            Tick::Passed => {
                c.move_to(2.0, 10.0);
                c.line_to(6.0, 14.0);
                c.line_to(14.0, 6.0);
            }
            Tick::Failed => {
                c.move_to(4.0, 6.0);
                c.line_to(12.0, 14.0);
                c.move_to(12.0, 6.0);
                c.line_to(4.0, 14.0);
            }
            Tick::Skipped => {
                c.move_to(3.0, 10.0);
                c.line_to(13.0, 10.0);
            }
            Tick::Unmarked | Tick::InProgress => c.arc(8.0, 10.0, 5.0, 0.0, std::f64::consts::TAU),
        }
        let _ = c.stroke();
    });
    area
}

/// Opens Evidence in the default app straight from the host. A layer-shell parent makes
/// Hyprland raise a protocol error that ends the daemon, and the portal never answers here
/// (`docs/facts.md`).
pub fn open(path: &std::path::Path) {
    gtk::gio::AppInfo::launch_default_for_uri_async(
        &gtk::gio::File::for_path(path).uri(),
        None::<&gtk::gio::AppLaunchContext>,
        None::<&gtk::gio::Cancellable>,
        |result| {
            if let Err(error) = result {
                tracing::warn!(%error, "Could not open Session Evidence");
            }
        },
    );
}

pub struct FooterActions {
    pub container: gtk::Box,
    control: gtk::Button,
    keycaps: gtk::Box,
    ended: Vec<(FooterButton, gtk::Button)>,
    action: std::rc::Rc<std::cell::Cell<Option<FooterButton>>>,
}
impl FooterActions {
    pub fn new(callback: impl Fn(FooterButton) + Clone + 'static) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let keycaps = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        keycaps.append(&label("Super", "keycap"));
        keycaps.append(&label("Esc", "keycap"));
        container.append(&keycaps);
        let control = gtk::Button::with_label("Take control");
        control.set_size_request(-1, 40);
        let action = std::rc::Rc::new(std::cell::Cell::new(None));
        let selected = action.clone();
        let callback_control = callback.clone();
        control.connect_clicked(move |_| {
            if let Some(kind) = selected.get() {
                callback_control(kind);
            }
        });
        container.append(&control);
        let mut ended = Vec::new();
        for kind in [
            FooterButton::OpenVideo,
            FooterButton::OpenFolder,
            FooterButton::Dismiss,
        ] {
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
            button.add_css_class(if kind == FooterButton::Dismiss {
                "dismiss"
            } else {
                "secondary"
            });
            button.update_property(&[gtk::accessible::Property::Label(kind.text())]);
            let action = callback.clone();
            button.connect_clicked(move |_| action(kind));
            container.append(&button);
            ended.push((kind, button));
        }
        Self {
            container,
            control,
            keycaps,
            ended,
            action,
        }
    }
    pub fn draw(&self, view: &FooterView) {
        self.action.set(view.button);
        self.control.set_visible(view.button.is_some());
        self.control
            .set_label(view.button.map_or("", FooterButton::text));
        self.keycaps.set_visible(view.keycaps);
        if view.label == super::footer_view::StateLabel::ViewOnly {
            self.control.add_css_class("hinted");
        } else {
            self.control.remove_css_class("hinted");
        }
        for (kind, button) in &self.ended {
            button.set_visible(view.buttons.contains(kind));
        }
    }
}
