use super::theme::Theme;
use gtk4 as gtk;

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
    provider.load_from_data(&format!("window.footer {{ background: {}; color: {}; font-family: 'JetBrains Mono'; font-size: 12px; border: 2px solid {}; border-top: 2px solid {}; border-radius: 0 0 8px 8px; }} window.violet {{border-top-color:{};}} .violet .cursor {{color:{};}} .cyan .cursor {{color:{};}} .dim {{color:{};}} .hint {{color:{};}} .value {{font-weight:700; font-feature-settings: 'tnum';}} button {{background:alpha({},0.10);color:{};border:1px solid {};border-radius:8px;font-weight:700;padding:0 18px;}} button:hover,button.hinted {{background:alpha({},0.22);}} window.hover-surface {{background:black;}} .card-edge {{border:2px solid {border};border-bottom:0;border-radius:8px 8px 0 0;}} .tick {{min-width:10px;min-height:10px;border-radius:2px;}} .tick.passed {{background:{};}} .tick.failed {{background:{};}} .tick.skipped {{border:1px solid {};color:{};}} .tick.unmarked {{border:1px solid {};}}",t.background,t.foreground,t.border,super::footer_view::SessionColour::Cyan.color(),super::footer_view::SessionColour::Violet.color(),super::footer_view::SessionColour::Violet.color(),super::footer_view::SessionColour::Cyan.color(),t.dim,t.accent,t.accent,t.accent,t.accent,t.accent,t.green,t.red,t.yellow,t.yellow,t.dim, border=t.border));
    let controls = gtk::CssProvider::new();
    controls.load_from_data(&format!("window.needs-you {{border-left-color:{0};border-right-color:{0};border-bottom-color:{0};}} .needs-you .card-edge {{border-color:{0};}} .takeover .card-edge {{border-color:{1};}} .needs-you .state {{color:{0};}} .needs-you button {{background:{0};color:{3};}} window.footer.takeover {{background:{4};border-left-color:{1};border-right-color:{1};border-bottom-color:{1};}} .takeover .state {{color:{5};}} .hand-back .state {{color:{5};}} .takeover button {{background:{6};color:{3};}} .keycap {{border:1px solid {2};border-radius:4px;padding:4px;}}", t.orange, t.active_border, t.border, t.ink, t.control, t.blue_text, t.accent));
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
