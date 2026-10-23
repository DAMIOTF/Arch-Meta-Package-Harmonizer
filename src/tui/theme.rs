use std::env;

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
}
