#[derive(Debug, Clone)]
pub struct Theme {
    pub background: String,
    pub foreground: String,
    pub dim: String,
    pub accent: String,
    pub border: String,
    pub green: String,
    pub red: String,
    pub yellow: String,
}
fn css_color(value: &str) -> Option<String> {
    let digits = value
        .strip_prefix('#')
        .or_else(|| value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')))
        .or_else(|| {
            value
                .strip_prefix("rgba(")
                .and_then(|v| v.strip_suffix(')'))
        })?;
    ((digits.len() == 6 || digits.len() == 8) && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{digits}"))
}
pub fn parse(input: &str) -> Theme {
    let values = input.parse::<toml::Table>().unwrap_or_default();
    let color = |key: &str, fallback: &str| {
        values
            .get(key)
            .and_then(toml::Value::as_str)
            .and_then(css_color)
            .unwrap_or_else(|| fallback.to_string())
    };
    Theme {
        background: color("background", "#193e58"),
        foreground: color("foreground", "#eee2cc"),
        dim: color("dark_foreground", "#c9c2b6"),
        accent: color("accent", "#77baf3"),
        border: color("hyprland_inactive_border", "#316184"),
        green: color("green", "#a9c972"),
        red: color("red", "#ff8e7c"),
        yellow: color("yellow", "#ebcf70"),
    }
}
fn state_home(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    xdg.map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|path| std::path::PathBuf::from(path).join(".local/state")))
}
pub fn load() -> Theme {
    let path = state_home(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
        .map(|path| path.join("omarchy/current/theme/colors.toml"));
    parse(
        &path
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default(),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_home_ignores_empty_and_relative_xdg_values() {
        for value in ["", "relative"] {
            assert_eq!(
                state_home(Some(value.into()), Some("/home/test".into())),
                Some(std::path::PathBuf::from("/home/test/.local/state"))
            );
        }
        assert_eq!(
            state_home(Some("/state".into()), Some("/home/test".into())),
            Some(std::path::PathBuf::from("/state"))
        );
    }
    #[test]
    fn missing_and_invalid_theme_uses_design_colors() {
        for input in [
            "",
            "not toml",
            "background = 42",
            "background = 'url(secret)'",
        ] {
            let t = parse(input);
            assert_eq!(t.background, "#193e58");
            assert_eq!(t.foreground, "#eee2cc");
        }
    }
    #[test]
    fn hyprland_border_colors_become_safe_css() {
        let t = parse("hyprland_inactive_border = 'rgba(123456aa)'\norange = '#abcdef'");
        assert_eq!(t.border, "#123456aa");
    }
    #[test]
    fn valid_keys_override_only_their_role() {
        let t = parse("background = '#123456'\nforeground = '#abcdef'\naccent = '#112233'");
        assert_eq!(t.background, "#123456");
        assert_eq!(t.foreground, "#abcdef");
        assert_eq!(t.dim, "#c9c2b6");
    }
}
