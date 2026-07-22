use std::time::Duration;

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};
use typst_tui_document::CursorPosition;
use typst_tui_theme::Theme;

use super::Component;
use crate::style::color;

pub(crate) struct StatusBarState {
    pub(crate) cursor: CursorPosition,
    pub(crate) selection_graphemes: usize,
    pub(crate) word_count: usize,
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) compile_time: Option<Duration>,
    pub(crate) message: Option<String>,
    pub(crate) quit_confirmation: bool,
}

pub(crate) struct StatusBar {
    state: StatusBarState,
    confirm_binding: String,
    cancel_binding: String,
    theme: Theme,
}

impl StatusBar {
    pub(crate) fn new(theme: Theme, confirm_binding: String, cancel_binding: String) -> Self {
        Self {
            state: StatusBarState {
                cursor: CursorPosition {
                    line: 0,
                    column: 0,
                    visual_column: 0,
                },
                selection_graphemes: 0,
                word_count: 0,
                errors: 0,
                warnings: 0,
                compile_time: None,
                message: None,
                quit_confirmation: false,
            },
            confirm_binding,
            cancel_binding,
            theme,
        }
    }

    pub(crate) fn set_state(&mut self, state: StatusBarState) {
        self.state = state;
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }
}

impl Component for StatusBar {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _focused: bool) {
        let compile_time = self.state.compile_time.map_or_else(
            || "--".to_owned(),
            |time| format!("{} ms", time.as_millis()),
        );
        let (message, foreground) = if self.state.quit_confirmation {
            (
                format!(
                    "Unsaved changes. {} quit, {} cancel",
                    self.confirm_binding, self.cancel_binding
                ),
                self.theme.warning,
            )
        } else if let Some(message) = &self.state.message {
            (message.clone(), self.theme.foreground)
        } else if self.state.selection_graphemes == 0 {
            (
                format!(
                    "Ln {}, Col {}",
                    self.state.cursor.line + 1,
                    self.state.cursor.column + 1
                ),
                self.theme.muted,
            )
        } else {
            (
                format!(
                    "Ln {}, Col {} | {} selected",
                    self.state.cursor.line + 1,
                    self.state.cursor.column + 1,
                    self.state.selection_graphemes
                ),
                self.theme.muted,
            )
        };
        let diagnostic_color = if self.state.errors > 0 {
            self.theme.error
        } else if self.state.warnings > 0 {
            self.theme.warning
        } else {
            self.theme.muted
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {message} | "),
                    Style::default().fg(color(foreground)),
                ),
                Span::styled(
                    format!(
                        "{} words | {} errors, {} warnings",
                        self.state.word_count, self.state.errors, self.state.warnings
                    ),
                    Style::default().fg(color(diagnostic_color)),
                ),
                Span::styled(
                    format!(" | {compile_time}"),
                    Style::default().fg(color(self.theme.muted)),
                ),
            ]))
            .style(Style::default().bg(color(self.theme.surface))),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use ratatui::{Terminal, backend::TestBackend};
    use typst_tui_document::CursorPosition;
    use typst_tui_theme::{ColorDepth, Theme, ThemeName};

    use super::{Component, StatusBar, StatusBarState};

    #[test]
    fn draws_editor_and_confirmation_state() -> Result<(), Infallible> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut status = StatusBar::new(theme, "alt+y".to_owned(), "alt+n".to_owned());
        status.set_state(StatusBarState {
            cursor: CursorPosition {
                line: 2,
                column: 4,
                visual_column: 4,
            },
            selection_graphemes: 0,
            word_count: 7,
            errors: 1,
            warnings: 0,
            compile_time: None,
            message: None,
            quit_confirmation: true,
        });
        let mut terminal = Terminal::new(TestBackend::new(90, 1))?;
        terminal.draw(|frame| status.draw(frame, frame.area(), false))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("alt+y quit, alt+n cancel"));
        assert!(rendered.contains("7 words | 1 errors"));
        Ok(())
    }
}
