#![forbid(unsafe_code)]

use std::{env, str::FromStr};

use thiserror::Error;
use typst_syntax::Tag;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorDepth {
    Ansi16,
    Ansi256,
    TrueColor,
}

impl ColorDepth {
    #[must_use]
    pub fn detect() -> Self {
        if env::var_os("WT_SESSION").is_some() {
            return Self::TrueColor;
        }
        Self::from_environment(
            env::var("TERM").ok().as_deref(),
            env::var("COLORTERM").ok().as_deref(),
        )
    }

    #[must_use]
    pub fn from_environment(term: Option<&str>, color_term: Option<&str>) -> Self {
        let term = term.map(str::to_ascii_lowercase);
        if color_term.is_some_and(|value| {
            value.eq_ignore_ascii_case("truecolor") || value.eq_ignore_ascii_case("24bit")
        }) || term.as_deref().is_some_and(|value| {
            value.contains("truecolor") || value.contains("24bit") || value.ends_with("-direct")
        }) {
            Self::TrueColor
        } else if term.is_some_and(|value| value.contains("256color")) {
            Self::Ansi256
        } else {
            Self::Ansi16
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Color {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    White,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextStyle {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub dim: bool,
}

impl TextStyle {
    #[must_use]
    pub fn patch(self, other: Self) -> Self {
        Self {
            foreground: other.foreground.or(self.foreground),
            background: other.background.or(self.background),
            bold: self.bold || other.bold,
            italic: self.italic || other.italic,
            underlined: self.underlined || other.underlined,
            dim: self.dim || other.dim,
        }
    }

    const fn foreground(color: Color) -> Self {
        Self {
            foreground: Some(color),
            background: None,
            bold: false,
            italic: false,
            underlined: false,
            dim: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeName {
    Dark,
    Light,
}

impl ThemeName {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

impl FromStr for ThemeName {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            _ => Err(Error::UnknownTheme(value.to_owned())),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("unknown theme `{0}`; expected `dark` or `light`")]
    UnknownTheme(String),
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub name: ThemeName,
    pub background: Color,
    pub foreground: Color,
    pub surface: Color,
    pub muted: Color,
    pub border: Color,
    pub accent: Color,
    pub selection: Color,
    pub current_line: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub info: Color,
}

impl Theme {
    #[must_use]
    pub fn new(name: ThemeName, depth: ColorDepth) -> Self {
        match (name, depth) {
            (ThemeName::Dark, ColorDepth::TrueColor) => Self::dark_rgb(),
            (ThemeName::Light, ColorDepth::TrueColor) => Self::light_rgb(),
            (ThemeName::Dark, ColorDepth::Ansi256) => Self::dark_indexed(),
            (ThemeName::Light, ColorDepth::Ansi256) => Self::light_indexed(),
            (ThemeName::Dark, ColorDepth::Ansi16) => Self::dark_ansi(),
            (ThemeName::Light, ColorDepth::Ansi16) => Self::light_ansi(),
        }
    }

    pub fn named(name: &str, depth: ColorDepth) -> Result<Self, Error> {
        Ok(Self::new(name.parse()?, depth))
    }

    #[must_use]
    pub fn syntax(self, tag: Tag) -> TextStyle {
        let foreground = |color| TextStyle::foreground(color);
        match tag {
            Tag::Comment => TextStyle {
                foreground: Some(self.muted),
                italic: true,
                ..TextStyle::default()
            },
            Tag::Punctuation => foreground(self.muted),
            Tag::Escape | Tag::MathDelimiter | Tag::MathOperator | Tag::MathGroupingParens => {
                foreground(self.accent)
            }
            Tag::Strong => TextStyle {
                bold: true,
                ..TextStyle::default()
            },
            Tag::Emph => TextStyle {
                italic: true,
                ..TextStyle::default()
            },
            Tag::Link => TextStyle {
                foreground: Some(self.info),
                underlined: true,
                ..TextStyle::default()
            },
            Tag::Raw | Tag::String => foreground(self.success),
            Tag::Label | Tag::Ref | Tag::Interpolated => foreground(self.info),
            Tag::Heading | Tag::ListMarker | Tag::ListTerm => TextStyle {
                foreground: Some(self.warning),
                bold: true,
                ..TextStyle::default()
            },
            Tag::Keyword => TextStyle {
                foreground: Some(self.accent),
                bold: true,
                ..TextStyle::default()
            },
            Tag::Operator => foreground(self.accent),
            Tag::Number | Tag::Function => foreground(self.info),
            Tag::Error => TextStyle {
                foreground: Some(self.error),
                underlined: true,
                ..TextStyle::default()
            },
        }
    }

    const fn dark_rgb() -> Self {
        Self {
            name: ThemeName::Dark,
            background: Color::Rgb(24, 24, 27),
            foreground: Color::Rgb(228, 228, 231),
            surface: Color::Rgb(39, 39, 42),
            muted: Color::Rgb(161, 161, 170),
            border: Color::Rgb(82, 82, 91),
            accent: Color::Rgb(34, 211, 238),
            selection: Color::Rgb(63, 63, 70),
            current_line: Color::Rgb(39, 39, 42),
            error: Color::Rgb(248, 113, 113),
            warning: Color::Rgb(250, 204, 21),
            success: Color::Rgb(74, 222, 128),
            info: Color::Rgb(96, 165, 250),
        }
    }

    const fn light_rgb() -> Self {
        Self {
            name: ThemeName::Light,
            background: Color::Rgb(250, 250, 250),
            foreground: Color::Rgb(39, 39, 42),
            surface: Color::Rgb(228, 228, 231),
            muted: Color::Rgb(82, 82, 91),
            border: Color::Rgb(161, 161, 170),
            accent: Color::Rgb(8, 145, 178),
            selection: Color::Rgb(207, 250, 254),
            current_line: Color::Rgb(244, 244, 245),
            error: Color::Rgb(185, 28, 28),
            warning: Color::Rgb(161, 98, 7),
            success: Color::Rgb(21, 128, 61),
            info: Color::Rgb(29, 78, 216),
        }
    }

    const fn dark_indexed() -> Self {
        Self {
            name: ThemeName::Dark,
            background: Color::Indexed(234),
            foreground: Color::Indexed(254),
            surface: Color::Indexed(237),
            muted: Color::Indexed(247),
            border: Color::Indexed(240),
            accent: Color::Indexed(44),
            selection: Color::Indexed(239),
            current_line: Color::Indexed(236),
            error: Color::Indexed(203),
            warning: Color::Indexed(220),
            success: Color::Indexed(77),
            info: Color::Indexed(75),
        }
    }

    const fn light_indexed() -> Self {
        Self {
            name: ThemeName::Light,
            background: Color::Indexed(231),
            foreground: Color::Indexed(235),
            surface: Color::Indexed(255),
            muted: Color::Indexed(240),
            border: Color::Indexed(247),
            accent: Color::Indexed(31),
            selection: Color::Indexed(195),
            current_line: Color::Indexed(255),
            error: Color::Indexed(124),
            warning: Color::Indexed(130),
            success: Color::Indexed(28),
            info: Color::Indexed(25),
        }
    }

    const fn dark_ansi() -> Self {
        Self {
            name: ThemeName::Dark,
            background: Color::Black,
            foreground: Color::White,
            surface: Color::DarkGray,
            muted: Color::Gray,
            border: Color::DarkGray,
            accent: Color::Cyan,
            selection: Color::DarkGray,
            current_line: Color::DarkGray,
            error: Color::Red,
            warning: Color::Yellow,
            success: Color::Green,
            info: Color::Blue,
        }
    }

    const fn light_ansi() -> Self {
        Self {
            name: ThemeName::Light,
            background: Color::White,
            foreground: Color::Black,
            surface: Color::Gray,
            muted: Color::DarkGray,
            border: Color::Gray,
            accent: Color::Cyan,
            selection: Color::Gray,
            current_line: Color::Gray,
            error: Color::Red,
            warning: Color::Yellow,
            success: Color::Green,
            info: Color::Blue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Color, ColorDepth, Theme, ThemeName};

    #[test]
    fn detects_common_color_capabilities() {
        assert_eq!(
            ColorDepth::from_environment(Some("xterm-256color"), None),
            ColorDepth::Ansi256
        );
        assert_eq!(
            ColorDepth::from_environment(Some("xterm"), Some("truecolor")),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::from_environment(Some("vt100"), None),
            ColorDepth::Ansi16
        );
        assert_eq!(
            ColorDepth::from_environment(Some("xterm-direct"), None),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::from_environment(Some("XTERM-24BIT"), None),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::from_environment(Some("xterm"), Some("TRUECOLOR")),
            ColorDepth::TrueColor
        );
        assert_eq!(ColorDepth::from_environment(None, None), ColorDepth::Ansi16);
    }

    #[test]
    fn ansi_theme_uses_only_portable_colors() {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        assert_eq!(theme.background, Color::Black);
        assert_eq!(theme.accent, Color::Cyan);
    }
}
