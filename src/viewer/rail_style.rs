use super::{footer_view::SessionColour, theme::Theme};
use std::fmt::Write as _;

pub(super) fn style(theme: &Theme) -> String {
    let mut css = format!(
        "window.rail-surface {{background:transparent;}}
         .rail-card, .rail-hover, .rail-locked-note {{
             background:{background};color:{foreground};font-family:'JetBrains Mono';font-size:12px;
             border-radius:8px;
         }}
         .rail-card {{border:2px solid {border};}}
         .rail-card.hovered {{background:{control};border-color:{active};}}
         .rail-card.needs-you {{border-color:{orange};}}
         .rail-card.locked {{opacity:0.55;}}
         .rail-still {{background:black;}}
         .rail-age, .rail-locked-note {{background:alpha({background},0.88);}}
         .rail-age {{font-size:11px;border-radius:4px;padding:3px 7px;}}
         .rail-text {{padding:14px 16px;}}
         .rail-session-edge {{min-height:2px;}}
         .rail-project {{font-size:13px;font-weight:700;}}
         .rail-needs-you {{color:{orange};font-weight:700;}}
         .rail-reason {{color:{foreground};}}
         .rail-hover {{border:2px solid {active};padding:16px 18px;}}
         .rail-check-mark {{min-width:12px;min-height:12px;border-radius:2px;font-size:10px;}}
         label.rail-check-mark.passed {{background:{green};color:{ink};}}
         label.rail-check-mark.failed {{background:{red};}}
         .rail-check-mark.skipped {{box-shadow:inset 0 0 0 1px {yellow};}}
         .rail-check-mark.unmarked, .rail-check-mark.checking {{box-shadow:inset 0 0 0 1px {dim};}}
         .rail-checking-pill {{
             color:{blue_text};font-size:11px;border:1px solid alpha({accent},0.65);
             border-radius:999px;padding:1px 7px;
         }}
         .rail-locked-note {{padding:10px 14px;}}",
        background = theme.background,
        foreground = theme.foreground,
        border = theme.border,
        control = theme.control,
        active = theme.active_border,
        orange = theme.orange,
        green = theme.green,
        ink = theme.ink,
        red = theme.red,
        yellow = theme.yellow,
        dim = theme.dim,
        blue_text = theme.blue_text,
        accent = theme.accent,
    );
    for colour in [
        SessionColour::Cyan,
        SessionColour::Violet,
        SessionColour::Pink,
        SessionColour::Green,
    ] {
        let class = colour.css_class();
        let color = colour.color();
        let _ = write!(
            css,
            "window.footer.{class} {{border-top-color:{color};}}
             .{class} .cursor {{color:{color};}}
             .{class} .rail-session-edge {{background:{color};}}"
        );
    }
    css
}
