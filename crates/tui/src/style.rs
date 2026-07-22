use ratatui::style::{Color as RatatuiColor, Modifier, Style};
use typst_tui_theme::{Color, TextStyle, Theme};

pub(crate) fn color(color: Color) -> RatatuiColor {
    match color {
        Color::Reset => RatatuiColor::Reset,
        Color::Black => RatatuiColor::Black,
        Color::Red => RatatuiColor::Red,
        Color::Green => RatatuiColor::Green,
        Color::Yellow => RatatuiColor::Yellow,
        Color::Blue => RatatuiColor::Blue,
        Color::Magenta => RatatuiColor::Magenta,
        Color::Cyan => RatatuiColor::Cyan,
        Color::Gray => RatatuiColor::Gray,
        Color::DarkGray => RatatuiColor::DarkGray,
        Color::White => RatatuiColor::White,
        Color::Indexed(index) => RatatuiColor::Indexed(index),
        Color::Rgb(red, green, blue) => RatatuiColor::Rgb(red, green, blue),
    }
}

pub(crate) fn text_style(style: TextStyle) -> Style {
    let mut result = Style::default();
    if let Some(foreground) = style.foreground {
        result = result.fg(color(foreground));
    }
    if let Some(background) = style.background {
        result = result.bg(color(background));
    }
    if style.bold {
        result = result.add_modifier(Modifier::BOLD);
    }
    if style.italic {
        result = result.add_modifier(Modifier::ITALIC);
    }
    if style.underlined {
        result = result.add_modifier(Modifier::UNDERLINED);
    }
    if style.dim {
        result = result.add_modifier(Modifier::DIM);
    }
    result
}

pub(crate) fn base(theme: &Theme) -> Style {
    Style::default()
        .fg(color(theme.foreground))
        .bg(color(theme.background))
}
