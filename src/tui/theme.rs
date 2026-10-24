use std::env;
use std::fmt;

use amph::plugin::ThemeSpec;
use ratatui::style::{Color, Modifier, Style};

/// The four intentional palettes plus the `NO_COLOR` degradation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Catppuccin,
    TokyoNight,
    Gruvbox,
    Nord,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 4] = [
        ThemeKind::Catppuccin,
        ThemeKind::TokyoNight,
        ThemeKind::Gruvbox,
        ThemeKind::Nord,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Catppuccin => "CATPPUCCIN",
            ThemeKind::TokyoNight => "TOKYO NIGHT",
            ThemeKind::Gruvbox => "GRUVBOX",
            ThemeKind::Nord => "NORD",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|kind| *kind == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        let index = Self::ALL.iter().position(|kind| *kind == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    fn from_name(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "catppuccin" | "mocha" => Some(Self::Catppuccin),
            "tokyo" | "tokyo-night" | "tokyonight" => Some(Self::TokyoNight),
            "gruvbox" => Some(Self::Gruvbox),
            "nord" => Some(Self::Nord),
            _ => None,
        }
    }
}

/// Which semantic field in a plugin theme could not be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeColorField {
    Accent,
    Good,
    Warn,
    Bad,
    Muted,
    Info,
}

impl ThemeColorField {
    pub fn label(self) -> &'static str {
        match self {
            ThemeColorField::Accent => "accent",
            ThemeColorField::Good => "good",
            ThemeColorField::Warn => "warn",
            ThemeColorField::Bad => "bad",
            ThemeColorField::Muted => "muted",
            ThemeColorField::Info => "info",
        }
    }
}

/// Why a validated plugin theme could not be applied to this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeApplyError {
    UnknownPalette,
    InvalidColor(ThemeColorField),
}

impl ThemeApplyError {
    pub fn message(self) -> String {
        match self {
            ThemeApplyError::UnknownPalette => {
                "The theme palette is not one of the built-in AMPH palettes.".to_string()
            }
            ThemeApplyError::InvalidColor(field) => format!(
                "The theme {} color is not a supported ANSI name or #RRGGBB value.",
                field.label()
            ),
        }
    }
}

impl fmt::Display for ThemeApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for ThemeApplyError {}

/// Parse the ANSI color names and exact `#RRGGBB` values promised by the
/// plugin manifest documentation. Matching is case-insensitive; anything else
/// is rejected.
pub fn parse_ansi_color(value: &str) -> Option<Color> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "gray" | "grey" => Some(Color::Gray),
        "dark gray" | "dark grey" | "bright black" => Some(Color::DarkGray),
        "bright red" => Some(Color::LightRed),
        "bright green" => Some(Color::LightGreen),
        "bright yellow" => Some(Color::LightYellow),
        "bright blue" => Some(Color::LightBlue),
        "bright magenta" => Some(Color::LightMagenta),
        "bright cyan" => Some(Color::LightCyan),
        "bright white" => Some(Color::White),
        _ => parse_hex_color(&value),
    }
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(red, green, blue))
}

/// Semantic colors for the whole interface.
///
/// Rendering code never uses a bare color literal. When `NO_COLOR` is present
/// every field becomes `Color::Reset`; layouts still read correctly through
/// borders, glyphs, and reversed/bold selection styles.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub kind: ThemeKind,
    pub no_color: bool,
    pub bg: Color,
    pub mantle: Color,
    pub surface: Color,
    pub surface_alt: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_alt: Color,
    pub positive: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,
    pub selection: Color,
    pub selection_fg: Color,
}

impl Theme {
    pub fn from_environment() -> Self {
        let kind = env::var("AMPH_THEME")
            .ok()
            .and_then(|value| ThemeKind::from_name(&value))
            .unwrap_or(ThemeKind::Catppuccin);
        Self::from_kind(kind)
    }

    /// Build a palette from an explicit theme while still honoring `NO_COLOR`.
    pub fn from_kind(kind: ThemeKind) -> Self {
        let no_color = env::var_os("NO_COLOR").is_some();
        Self::new(kind, no_color)
    }

    /// Apply a validated plugin [`ThemeSpec`] to the current session.
    ///
    /// The built-in palette named by `spec.palette` supplies the layout colors,
    /// then `accent`, `good`, `warn`, `bad`, `muted`, and `info` replace their
    /// semantic counterparts. Every color is parsed strictly: only the
    /// documented ANSI names and exact `#RRGGBB` values are accepted. `NO_COLOR`
    /// still wins and leaves every color reset; parsing still rejects malformed
    /// values so the result does not depend on the environment.
    pub fn from_plugin_spec(spec: &ThemeSpec) -> Result<Self, ThemeApplyError> {
        Self::from_plugin_spec_with_no_color(spec, env::var_os("NO_COLOR").is_some())
    }

    pub(crate) fn from_plugin_spec_with_no_color(
        spec: &ThemeSpec,
        no_color: bool,
    ) -> Result<Self, ThemeApplyError> {
        let kind = ThemeKind::from_name(&spec.palette).ok_or(ThemeApplyError::UnknownPalette)?;
        let accent = parse_ansi_color(&spec.accent)
            .ok_or(ThemeApplyError::InvalidColor(ThemeColorField::Accent))?;
        let good = parse_ansi_color(&spec.good)
            .ok_or(ThemeApplyError::InvalidColor(ThemeColorField::Good))?;
        let warn = parse_ansi_color(&spec.warn)
            .ok_or(ThemeApplyError::InvalidColor(ThemeColorField::Warn))?;
        let bad = parse_ansi_color(&spec.bad)
            .ok_or(ThemeApplyError::InvalidColor(ThemeColorField::Bad))?;
        let muted = parse_ansi_color(&spec.muted)
            .ok_or(ThemeApplyError::InvalidColor(ThemeColorField::Muted))?;
        let info = parse_ansi_color(&spec.info)
            .ok_or(ThemeApplyError::InvalidColor(ThemeColorField::Info))?;

        let mut theme = Self::new(kind, no_color);
        if no_color {
            return Ok(theme);
        }
        theme.accent = accent;
        theme.positive = good;
        theme.warning = warn;
        theme.danger = bad;
        theme.muted = muted;
        theme.info = info;
        Ok(theme)
    }

    pub fn new(kind: ThemeKind, no_color: bool) -> Self {
        let mut theme = match kind {
            ThemeKind::Catppuccin => Self {
                kind,
                no_color: false,
                bg: rgb(0x1e, 0x1e, 0x2e),
                mantle: rgb(0x18, 0x18, 0x25),
                surface: rgb(0x31, 0x32, 0x44),
                surface_alt: rgb(0x45, 0x47, 0x5a),
                border: rgb(0x58, 0x5b, 0x70),
                text: rgb(0xcd, 0xd6, 0xf4),
                muted: rgb(0x7f, 0x84, 0x9c),
                accent: rgb(0xcb, 0xa6, 0xf7),
                accent_alt: rgb(0x89, 0xb4, 0xfa),
                positive: rgb(0xa6, 0xe3, 0xa1),
                warning: rgb(0xf9, 0xe2, 0xaf),
                danger: rgb(0xf3, 0x8b, 0xa8),
                info: rgb(0x94, 0xe2, 0xd5),
                selection: rgb(0x45, 0x47, 0x5a),
                selection_fg: rgb(0xf5, 0xe0, 0xdc),
            },
            ThemeKind::TokyoNight => Self {
                kind,
                no_color: false,
                bg: rgb(0x1a, 0x1b, 0x26),
                mantle: rgb(0x16, 0x16, 0x1e),
                surface: rgb(0x24, 0x28, 0x3b),
                surface_alt: rgb(0x2f, 0x35, 0x49),
                border: rgb(0x3b, 0x42, 0x61),
                text: rgb(0xc0, 0xca, 0xf5),
                muted: rgb(0x56, 0x5f, 0x89),
                accent: rgb(0xbb, 0x9a, 0xf7),
                accent_alt: rgb(0x7a, 0xa2, 0xf7),
                positive: rgb(0x9e, 0xce, 0x6a),
                warning: rgb(0xe0, 0xaf, 0x68),
                danger: rgb(0xf7, 0x76, 0x8e),
                info: rgb(0x7d, 0xcf, 0xff),
                selection: rgb(0x33, 0x46, 0x7c),
                selection_fg: rgb(0xc0, 0xca, 0xf5),
            },
            ThemeKind::Gruvbox => Self {
                kind,
                no_color: false,
                bg: rgb(0x28, 0x28, 0x28),
                mantle: rgb(0x1d, 0x20, 0x21),
                surface: rgb(0x32, 0x30, 0x2f),
                surface_alt: rgb(0x3c, 0x38, 0x36),
                border: rgb(0x50, 0x49, 0x45),
                text: rgb(0xeb, 0xdb, 0xb2),
                muted: rgb(0x92, 0x83, 0x74),
                accent: rgb(0xfe, 0x80, 0x19),
                accent_alt: rgb(0x83, 0xa5, 0x98),
                positive: rgb(0xb8, 0xbb, 0x26),
                warning: rgb(0xfa, 0xbd, 0x2f),
                danger: rgb(0xfb, 0x49, 0x34),
                info: rgb(0x8e, 0xc0, 0x7c),
                selection: rgb(0x50, 0x49, 0x45),
                selection_fg: rgb(0xfb, 0xf1, 0xc7),
            },
            ThemeKind::Nord => Self {
                kind,
                no_color: false,
                bg: rgb(0x2e, 0x34, 0x40),
                mantle: rgb(0x27, 0x2c, 0x36),
                surface: rgb(0x3b, 0x42, 0x52),
                surface_alt: rgb(0x43, 0x4c, 0x5e),
                border: rgb(0x4c, 0x56, 0x6a),
                text: rgb(0xec, 0xef, 0xf4),
                muted: rgb(0x7b, 0x88, 0xa1),
                accent: rgb(0x88, 0xc0, 0xd0),
                accent_alt: rgb(0x81, 0xa1, 0xc1),
                positive: rgb(0xa3, 0xbe, 0x8c),
                warning: rgb(0xeb, 0xcb, 0x8b),
                danger: rgb(0xbf, 0x61, 0x6a),
                info: rgb(0x8f, 0xbc, 0xbb),
                selection: rgb(0x4c, 0x56, 0x6a),
                selection_fg: rgb(0xec, 0xef, 0xf4),
            },
        };

        if no_color {
            theme.no_color = true;
            theme.bg = Color::Reset;
            theme.mantle = Color::Reset;
            theme.surface = Color::Reset;
            theme.surface_alt = Color::Reset;
            theme.border = Color::Reset;
            theme.text = Color::Reset;
            theme.muted = Color::Reset;
            theme.accent = Color::Reset;
            theme.accent_alt = Color::Reset;
            theme.positive = Color::Reset;
            theme.warning = Color::Reset;
            theme.danger = Color::Reset;
            theme.info = Color::Reset;
            theme.selection = Color::Reset;
            theme.selection_fg = Color::Reset;
        }

        theme
    }

    pub fn next(&self) -> Self {
        Self::new(self.kind.next(), self.no_color)
    }

    pub fn previous(&self) -> Self {
        Self::new(self.kind.previous(), self.no_color)
    }

    pub fn base_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.bg)
    }

    pub fn mantle_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.mantle)
    }

    pub fn panel_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.surface)
    }

    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn text_style(&self) -> Style {
        Style::default().fg(self.text)
    }

    pub fn accent_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn selection_style(&self) -> Style {
        if self.no_color {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.selection_fg)
                .bg(self.selection)
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn tab_active_style(&self) -> Style {
        if self.no_color {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.text)
                .bg(self.surface_alt)
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn tab_inactive_style(&self) -> Style {
        if self.no_color {
            Style::default()
        } else {
            Style::default().fg(self.muted)
        }
    }

    pub fn badge_style(&self, color: Color) -> Style {
        if self.no_color {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(color)
                .bg(self.surface_alt)
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn score_color(&self, score: u8) -> Color {
        if score >= 75 {
            self.positive
        } else if score >= 45 {
            self.warning
        } else {
            self.danger
        }
    }

    pub fn score_style(&self, score: u8) -> Style {
        Style::default()
            .fg(self.score_color(score))
            .add_modifier(Modifier::BOLD)
    }

    pub(crate) fn status_color(&self, status: crate::tui::metrics::CheckStatus) -> Color {
        use crate::tui::metrics::CheckStatus;
        match status {
            CheckStatus::Pass => self.positive,
            CheckStatus::Info => self.info,
            CheckStatus::Warn => self.warning,
            CheckStatus::Fail => self.danger,
        }
    }

    pub fn is_monochrome(&self) -> bool {
        self.no_color
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_cycle_and_wrap() {
        assert_eq!(ThemeKind::Catppuccin.next(), ThemeKind::TokyoNight);
        assert_eq!(ThemeKind::Catppuccin.previous(), ThemeKind::Nord);
    }

    #[test]
    fn no_color_resets_every_color() {
        let theme = Theme::new(ThemeKind::Gruvbox, true);
        assert_eq!(theme.accent, Color::Reset);
        assert_eq!(theme.bg, Color::Reset);
        assert!(theme.is_monochrome());
    }

    #[test]
    fn ansi_color_names_and_exact_hex_are_supported() {
        for (name, color) in [
            ("black", Color::Black),
            ("RED", Color::Red),
            ("Green", Color::Green),
            ("yellow", Color::Yellow),
            ("blue", Color::Blue),
            ("magenta", Color::Magenta),
            ("cyan", Color::Cyan),
            ("white", Color::White),
            ("gray", Color::Gray),
            ("grey", Color::Gray),
            ("dark gray", Color::DarkGray),
            ("dark grey", Color::DarkGray),
            ("bright black", Color::DarkGray),
            ("bright red", Color::LightRed),
            ("bright green", Color::LightGreen),
            ("bright yellow", Color::LightYellow),
            ("bright blue", Color::LightBlue),
            ("bright magenta", Color::LightMagenta),
            ("bright cyan", Color::LightCyan),
            ("bright white", Color::White),
        ] {
            assert_eq!(parse_ansi_color(name), Some(color), "{name}");
        }
        assert_eq!(
            parse_ansi_color("#112233"),
            Some(Color::Rgb(0x11, 0x22, 0x33))
        );
        assert_eq!(
            parse_ansi_color("#aAbBcC"),
            Some(Color::Rgb(0xaa, 0xbb, 0xcc))
        );
    }

    #[test]
    fn unsupported_colors_are_rejected() {
        for value in [
            "",
            "orange",
            "light red",
            "default",
            "reset",
            "16",
            "#123",
            "#12345",
            "#1234567",
            "#GG0000",
            "#11223G",
            "bright  red",
        ] {
            assert_eq!(parse_ansi_color(value), None, "{value}");
        }
    }

    #[test]
    fn plugin_theme_applies_palette_and_semantic_overrides() {
        let spec = ThemeSpec {
            palette: "nord".to_string(),
            accent: "#112233".to_string(),
            good: "green".to_string(),
            warn: "bright yellow".to_string(),
            bad: "red".to_string(),
            muted: "dark gray".to_string(),
            info: "bright cyan".to_string(),
        };
        let theme = Theme::from_plugin_spec_with_no_color(&spec, false).expect("theme");
        assert_eq!(theme.kind, ThemeKind::Nord);
        assert_eq!(theme.accent, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(theme.positive, Color::Green);
        assert_eq!(theme.warning, Color::LightYellow);
        assert_eq!(theme.danger, Color::Red);
        assert_eq!(theme.muted, Color::DarkGray);
        assert_eq!(theme.info, Color::LightCyan);
    }

    #[test]
    fn plugin_theme_honors_no_color() {
        let spec = ThemeSpec {
            palette: "gruvbox".to_string(),
            accent: "#112233".to_string(),
            ..ThemeSpec::default()
        };
        let theme = Theme::from_plugin_spec_with_no_color(&spec, true).expect("theme");
        assert!(theme.is_monochrome());
        assert_eq!(theme.kind, ThemeKind::Gruvbox);
        for color in [
            theme.bg,
            theme.text,
            theme.accent,
            theme.positive,
            theme.warning,
            theme.danger,
            theme.info,
            theme.muted,
        ] {
            assert_eq!(color, Color::Reset);
        }
    }

    #[test]
    fn plugin_theme_rejects_unknown_palette_and_bad_color() {
        let spec = ThemeSpec {
            palette: "unknown".to_string(),
            ..ThemeSpec::default()
        };
        assert!(matches!(
            Theme::from_plugin_spec_with_no_color(&spec, false),
            Err(ThemeApplyError::UnknownPalette)
        ));

        let spec = ThemeSpec {
            accent: "orange".to_string(),
            ..ThemeSpec::default()
        };
        assert!(matches!(
            Theme::from_plugin_spec_with_no_color(&spec, false),
            Err(ThemeApplyError::InvalidColor(ThemeColorField::Accent))
        ));
    }
}
