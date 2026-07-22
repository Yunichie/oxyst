use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use typst_tui_theme::{Color, Theme};

use super::Component;
use crate::style::color;

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
        let dirty = if self.state.dirty { " ●" } else { "" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " typst-tui ",
                    Style::default()
                        .fg(color(self.theme.accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{}{}", self.state.display_name, dirty)),
                Span::styled(
                    format!(" | {}", self.state.compile_label),
                    Style::default().fg(color(self.state.compile_color)),
                ),
                Span::styled(
                    format!(" | {} help", self.help_binding),
                    Style::default().fg(color(self.theme.muted)),
                ),
            ])),
            area,
        );
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
}
