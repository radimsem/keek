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
    pub fn new(snapshot: &SessionSnapshot, width: i32) -> Option<Self> {
        let frame = match &snapshot.state {
            SessionState::Rendering { frame, .. } => frame,
            SessionState::Ended(end) => &end.frame,
            SessionState::Opening | SessionState::Open | SessionState::Closing => return None,
        };
        let overlay = gtk::Overlay::new();
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
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
        overlay.set_child(Some(&picture));
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("end-scrim");
        overlay.add_overlay(&scrim);
        let view = verdict_view::for_snapshot(snapshot)?;
        let (panel, progress, tally) = panel(view, width);
        overlay.add_overlay(&panel);
        Some(Self {
            overlay,
            panel,
            progress,
            tally,
            width,
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
    surface: &gtk::Window,
    snapshot: &SessionSnapshot,
    width: i32,
) {
    match card {
        Some(end) => end.update(snapshot),
        None => {
            *card = EndCard::new(snapshot, width);
            surface.set_child(card.as_ref().map(|end| &end.overlay));
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
                Tick::Unmarked => &theme.dim,
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
            Tick::Unmarked => c.arc(8.0, 10.0, 5.0, 0.0, std::f64::consts::TAU),
        }
        let _ = c.stroke();
    });
    area
}

pub fn open(path: &std::path::Path, parent: &gtk::Window) {
    gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(path))).launch(
        Some(parent),
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

pub fn style(theme: &Theme) -> String {
    format!(
        ".end-scrim {{background:alpha({ink},0.86);}} .verdict-panel {{background:{background};color:{foreground};font-family:'JetBrains Mono';border:1px solid {border};border-radius:8px;padding:28px 32px;}} .verdict-title {{font-size:22px;font-weight:700;}} .panel-title {{font-size:18px;font-weight:700;font-feature-settings:'tnum';}} .panel-meta {{font-size:13px;color:{dim};}} .check-text {{font-size:14px;}} .check-comment {{font-size:13px;color:mix({foreground},{dim},0.5);}} .added-tag {{color:{dim};font-size:11px;border:1px solid {border};border-radius:4px;padding:1px 6px;}} .verdict-panel separator {{background:{border};min-height:1px;}} .verdict-panel progressbar trough {{min-height:6px;background:alpha({foreground},0.2);}} .verdict-panel progressbar progress {{min-height:6px;background:{accent};}} label.passed {{color:{green};background:transparent;}} label.failed {{color:{red};background:transparent;}} label.skipped {{color:{yellow};border:0;}} .tick.passed {{background:{green};}} .tick.failed {{background:{red};}} button.secondary {{background:alpha({foreground},0.04);color:{foreground};border:1px solid alpha({foreground},0.4);padding:0 16px;}} button.dismiss {{background:transparent;color:{dim};border:0;padding:0;}}",
        ink = theme.ink,
        background = theme.background,
        foreground = theme.foreground,
        border = theme.border,
        dim = theme.dim,
        accent = theme.accent,
        green = theme.green,
        red = theme.red,
        yellow = theme.yellow
    )
}

pub fn install_style() {
    let t = super::theme::load();
    let provider = gtk::CssProvider::new();
    provider.load_from_data(&format!("window.footer {{ background: {}; color: {}; font-family: 'JetBrains Mono'; font-size: 12px; border: 2px solid {}; border-top: 2px solid {}; border-radius: 0 0 8px 8px; }} window.violet {{border-top-color:{};}} .violet .cursor {{color:{};}} .cyan .cursor {{color:{};}} .dim {{color:{};}} .hint {{color:{};}} .value {{font-weight:700; font-feature-settings: 'tnum';}} button {{background:alpha({},0.10);color:{};border:1px solid {};border-radius:8px;font-weight:700;padding:0 18px;}} button:hover,button.hinted {{background:alpha({},0.22);}} window.hover-surface {{background:transparent;}} .tick {{min-width:10px;min-height:10px;border-radius:2px;}} .tick.passed {{background:{};}} .tick.failed {{background:{};}} .tick.skipped {{border:1px solid {};color:{};}} .tick.unmarked {{border:1px solid {};}}",t.background,t.foreground,t.border,super::footer_view::SessionColour::Cyan.color(),super::footer_view::SessionColour::Violet.color(),super::footer_view::SessionColour::Violet.color(),super::footer_view::SessionColour::Cyan.color(),t.dim,t.accent,t.accent,t.accent,t.accent,t.accent,t.green,t.red,t.yellow,t.yellow,t.dim));
    let controls = gtk::CssProvider::new();
    controls.load_from_data(&format!("window.needs-you {{border-left-color:{0};border-right-color:{0};border-bottom-color:{0};}} .needs-you .state {{color:{0};}} .needs-you button {{background:{0};color:{3};}} window.takeover {{background:{4};border-left-color:{1};border-right-color:{1};border-bottom-color:{1};}} .takeover .state {{color:{5};}} .hand-back .state {{color:{5};}} .takeover button {{background:{6};color:{3};}} .keycap {{border:1px solid {2};border-radius:4px;padding:4px;}}", t.orange, t.active_border, t.border, t.ink, t.control, t.blue_text, t.accent));
    let end_style = gtk::CssProvider::new();
    end_style.load_from_data(&style(&t));
    // GTK breaks ties between equal-priority providers by the order they were added, so `controls` goes last.
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &end_style,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &controls,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
