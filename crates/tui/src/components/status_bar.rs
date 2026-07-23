use std::time::Duration;

use oxyst_document::CursorPosition;
use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::Component;
use crate::style::{color, truncate};

const MIN_MESSAGE_WIDTH: usize = 12;

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
        if area.width == 0 {
            return;
        }
        frame.render_widget(
            Block::default().style(Style::default().bg(color(self.theme.surface))),
            area,
        );

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
        let diagnostics = format!(
            "{} errors, {} warnings",
            self.state.errors, self.state.warnings
        );
        let full_prefix = format!(" | {} words | ", self.state.word_count);
        let full_suffix = format!(" | {compile_time}");
        let full_width = UnicodeWidthStr::width(full_prefix.as_str())
            + UnicodeWidthStr::width(diagnostics.as_str())
            + UnicodeWidthStr::width(full_suffix.as_str());
        let diagnostic_prefix = " | ";
        let diagnostic_width = UnicodeWidthStr::width(diagnostic_prefix)
            + UnicodeWidthStr::width(diagnostics.as_str());
        let compact = format!("{}E {}W", self.state.errors, self.state.warnings);
        let compact_width = UnicodeWidthStr::width(compact.as_str());
        let available = usize::from(area.width);
        let (details, details_width) = if full_width + MIN_MESSAGE_WIDTH <= available {
            (
                Line::from(vec![
                    Span::styled(full_prefix, Style::default().fg(color(self.theme.muted))),
                    Span::styled(diagnostics, Style::default().fg(color(diagnostic_color))),
                    Span::styled(full_suffix, Style::default().fg(color(self.theme.muted))),
                ]),
                full_width,
            )
        } else if diagnostic_width + MIN_MESSAGE_WIDTH <= available {
            (
                Line::from(vec![
                    Span::styled(
                        diagnostic_prefix,
                        Style::default().fg(color(self.theme.muted)),
                    ),
                    Span::styled(diagnostics, Style::default().fg(color(diagnostic_color))),
                ]),
                diagnostic_width,
            )
        } else if compact_width <= available {
            (
                Line::styled(compact, Style::default().fg(color(diagnostic_color))),
                compact_width,
            )
        } else {
            let critical = if self.state.errors > 0 {
                format!("E{}", self.state.errors)
            } else if self.state.warnings > 0 {
                format!("W{}", self.state.warnings)
            } else {
                "OK".to_owned()
            };
            let critical = critical.chars().take(available).collect::<String>();
            let width = UnicodeWidthStr::width(critical.as_str());
            (
                Line::styled(critical, Style::default().fg(color(diagnostic_color))),
                width,
            )
        };
        let message_width = available.saturating_sub(details_width);
        let message = truncate(&format!(" {message}"), message_width);
        let message_area = Rect::new(area.x, area.y, message_width as u16, area.height);
        let details_area = Rect::new(
            message_area.right(),
            area.y,
            details_width as u16,
            area.height,
        );

        frame.render_widget(
            Paragraph::new(message).style(
                Style::default()
                    .fg(color(foreground))
                    .bg(color(self.theme.surface)),
            ),
            message_area,
        );
        frame.render_widget(
            Paragraph::new(details).style(Style::default().bg(color(self.theme.surface))),
            details_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, time::Duration};

    use oxyst_document::CursorPosition;
    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend};

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

    #[test]
    fn keeps_diagnostics_visible_with_a_long_message() -> Result<(), Infallible> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut status = StatusBar::new(theme, "y".to_owned(), "n".to_owned());
        status.set_state(StatusBarState {
            cursor: CursorPosition {
                line: 0,
                column: 0,
                visual_column: 0,
            },
            selection_graphemes: 0,
            word_count: 1234,
            errors: 12,
            warnings: 3,
            compile_time: Some(Duration::from_millis(42)),
            message: Some("a diagnostic message that is much wider than the terminal".to_owned()),
            quit_confirmation: false,
        });
        let mut terminal = Terminal::new(TestBackend::new(32, 1))?;
        terminal.draw(|frame| status.draw(frame, frame.area(), false))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("12E 3W"));
        assert!(rendered.contains('…'));
        Ok(())
    }
}
