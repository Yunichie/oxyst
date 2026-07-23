use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use typst_tui_theme::{Color, Theme};
use unicode_width::UnicodeWidthStr;

use super::Component;
use crate::style::{color, truncate};

const MIN_DOCUMENT_WIDTH_WITH_HELP: usize = 16;

pub(crate) struct HeaderState {
    pub(crate) display_name: String,
    pub(crate) dirty: bool,
    pub(crate) compile_label: String,
    pub(crate) compile_color: Color,
}

pub(crate) struct Header {
    state: HeaderState,
    help_binding: String,
    theme: Theme,
}

impl Header {
    pub(crate) fn new(theme: Theme, help_binding: String) -> Self {
        Self {
            state: HeaderState {
                display_name: "Untitled".to_owned(),
                dirty: false,
                compile_label: "not compiled".to_owned(),
                compile_color: theme.muted,
            },
            help_binding,
            theme,
        }
    }

    pub(crate) fn set_state(&mut self, state: HeaderState) {
        self.state = state;
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }
}

impl Component for Header {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _focused: bool) {
        if area.width == 0 {
            return;
        }

        let compile = format!(" | {}", self.state.compile_label);
        let help = format!(" | {} help", self.help_binding);
        let width = usize::from(area.width);
        let compile_width = UnicodeWidthStr::width(compile.as_str()).min(width);
        let help_width = UnicodeWidthStr::width(help.as_str());
        let show_help = compile_width + help_width + MIN_DOCUMENT_WIDTH_WITH_HELP <= width;
        let right_width = compile_width + if show_help { help_width } else { 0 };
        let left_width = width.saturating_sub(right_width);
        let left_area = Rect::new(area.x, area.y, left_width as u16, area.height);
        let compile_area = Rect::new(left_area.right(), area.y, compile_width as u16, area.height);

        self.draw_document(frame, left_area);
        frame.render_widget(
            Paragraph::new(truncate(&compile, compile_width))
                .style(Style::default().fg(color(self.state.compile_color))),
            compile_area,
        );
        if show_help {
            frame.render_widget(
                Paragraph::new(help).style(Style::default().fg(color(self.theme.muted))),
                Rect::new(compile_area.right(), area.y, help_width as u16, area.height),
            );
        }
    }
}

impl Header {
    fn draw_document(&self, frame: &mut Frame, area: Rect) {
        let width = usize::from(area.width);
        if width == 0 {
            return;
        }

        let dirty = if self.state.dirty { " ●" } else { "" };
        let dirty_width = UnicodeWidthStr::width(dirty).min(width);
        let identity_width = width.saturating_sub(dirty_width);
        let brand = " typst-tui ";
        let brand_width = UnicodeWidthStr::width(brand);
        let identity = if identity_width > brand_width {
            Line::from(vec![
                Span::styled(
                    brand,
                    Style::default()
                        .fg(color(self.theme.accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(truncate(
                    &self.state.display_name,
                    identity_width - brand_width,
                )),
            ])
        } else {
            Line::raw(truncate(&self.state.display_name, identity_width))
        };
        let identity_rendered_width = identity.width().min(identity_width);
        frame.render_widget(
            Paragraph::new(identity),
            Rect::new(area.x, area.y, identity_width as u16, area.height),
        );
        if dirty_width > 0 {
            frame.render_widget(
                Paragraph::new(truncate(dirty, dirty_width)),
                Rect::new(
                    area.x.saturating_add(identity_rendered_width as u16),
                    area.y,
                    dirty_width as u16,
                    area.height,
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use ratatui::{Terminal, backend::TestBackend};
    use typst_tui_theme::{ColorDepth, Theme, ThemeName};

    use super::{Component, Header, HeaderState};

    #[test]
    fn draws_document_and_configured_help_binding() -> Result<(), Infallible> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut header = Header::new(theme, "alt+h".to_owned());
        header.set_state(HeaderState {
            display_name: "report.typ".to_owned(),
            dirty: true,
            compile_label: "● compiling".to_owned(),
            compile_color: theme.warning,
        });
        let mut terminal = Terminal::new(TestBackend::new(64, 1))?;
        terminal.draw(|frame| header.draw(frame, frame.area(), false))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("report.typ ●"));
        assert!(rendered.contains("alt+h help"));
        Ok(())
    }

    #[test]
    fn keeps_compile_state_visible_with_a_long_filename() -> Result<(), Infallible> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut header = Header::new(theme, "f1".to_owned());
        header.set_state(HeaderState {
            display_name: "a-very-long-界-document-name.typ".to_owned(),
            dirty: true,
            compile_label: "✕ 12 errors".to_owned(),
            compile_color: theme.error,
        });
        let mut terminal = Terminal::new(TestBackend::new(30, 1))?;
        terminal.draw(|frame| header.draw(frame, frame.area(), false))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("✕ 12 errors"));
        assert!(rendered.contains('●'));
        assert!(!rendered.contains("f1 help"));
        Ok(())
    }
}
