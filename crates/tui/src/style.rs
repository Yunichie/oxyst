use oxyst_theme::{Color, TextStyle, Theme};
use ratatui::style::{Color as RatatuiColor, Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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

pub(crate) fn truncate(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }

    let content_width = width - 1;
    let mut result = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used + grapheme_width > content_width {
            break;
        }
        result.push_str(grapheme);
        used += grapheme_width;
    }
    result.push('…');
    result
}
