use super::{
    footer_view::Tick,
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
    geometry: crate::slots::CardGeometry,
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
        let (panel, progress, tally) = panel(view, geometry);
        overlay.add_overlay(&panel);
        Some(Self {
            overlay,
            panel,
            progress,
            tally,
            geometry,
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
                    let (panel, progress, tally) = panel(view, self.geometry);
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
fn panel_width(mode: verdict_view::PanelMode, geometry: crate::slots::CardGeometry) -> i32 {
    match mode {
        verdict_view::PanelMode::Rendering(_) => 560,
        verdict_view::PanelMode::Ended => 720,
    }
    .min((geometry.window.width - 32).max(1))
}
fn panel(
    view: verdict_view::PanelView,
    geometry: crate::slots::CardGeometry,
) -> (gtk::Box, Option<gtk::ProgressBar>, gtk::Label) {
    let theme = super::theme::load();
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 20);
    panel.add_css_class("verdict-panel");
    panel.set_halign(gtk::Align::Center);
    panel.set_margin_start(16);
    panel.set_margin_end(16);
    panel.set_valign(gtk::Align::Center);
    panel.set_size_request(panel_width(view.mode, geometry), -1);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ipc::Percent, slots::Slot};
    fn card(window: (i32, i32)) -> crate::slots::CardGeometry {
        crate::slots::geometry(
            Slot {
                x: 24,
                y: 50,
                width: 1712,
                height: 1300,
            },
            Some(window),
        )
    }
    #[test]
    fn panels_are_never_wider_than_a_window_smaller_than_the_slot() {
        let geometry = card((390, 844));
        for mode in [
            verdict_view::PanelMode::Rendering(Percent(40)),
            verdict_view::PanelMode::Ended,
        ] {
            assert!(panel_width(mode, geometry) <= geometry.window.width);
        }
    }
    #[test]
    fn panels_keep_their_own_width_in_a_window_that_fills_the_slot() {
        let geometry = card((1712, 1300));
        assert_eq!(
            panel_width(verdict_view::PanelMode::Rendering(Percent(40)), geometry),
            560
        );
        assert_eq!(panel_width(verdict_view::PanelMode::Ended, geometry), 720);
    }
}
