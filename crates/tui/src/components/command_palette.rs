use oxyst_config::CommandId;
use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::style::{base, color};

use super::modal_area;

#[derive(Debug, Default)]
pub(crate) struct CommandPalette {
    query: String,
    selected: usize,
}

impl CommandPalette {
    pub(crate) fn input(&mut self, character: char) {
        self.query.push(character);
        self.selected = 0;
    }

    pub(crate) fn input_text(&mut self, text: &str) {
        self.query.push_str(text);
        self.selected = 0;
    }

    pub(crate) fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub(crate) fn move_selection(&mut self, direction: isize) {
        let count = self.matches().len();
        if count == 0 {
            self.selected = 0;
        } else {
            self.selected = self
                .selected
                .saturating_add_signed(direction)
                .min(count - 1);
        }
    }

    pub(crate) fn selected(&self) -> Option<CommandId> {
        self.matches().get(self.selected).copied()
    }

    pub(crate) fn draw(&self, frame: &mut Frame, theme: &Theme) {
        let area = modal_area(frame.area(), 60, 16);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(theme.accent)))
            .style(base(theme))
            .title(" Command palette ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let [query_area, results_area] =
            Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(inner);
        let available = usize::from(results_area.height);
        let matches = self.matches();
        let start = visible_start(self.selected, matches.len(), available);
        let lines = matches
            .into_iter()
            .enumerate()
            .skip(start)
            .take(available)
            .map(|(index, command)| {
                let line = Line::raw(format!("  {}", command.label()));
                if index == self.selected {
                    line.style(
                        Style::default()
                            .bg(color(theme.selection))
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    line
                }
            })
            .collect::<Vec<_>>();
        let cursor = u16::try_from(UnicodeWidthStr::width(self.query.as_str())).unwrap_or(u16::MAX);
        let visible_width = inner.width.saturating_sub(2).max(1);
        let scroll = cursor.saturating_sub(visible_width.saturating_sub(1));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(color(theme.accent))),
                Span::raw(&self.query),
            ]))
            .scroll((0, scroll)),
            query_area,
        );
        frame.render_widget(Paragraph::new(lines), results_area);
        frame.set_cursor_position((
            inner
                .x
                .saturating_add(2)
                .saturating_add(cursor.saturating_sub(scroll))
                .min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }

    fn matches(&self) -> Vec<CommandId> {
        CommandId::all()
            .filter(|command| command.show_in_palette())
            .filter(|command| fuzzy_match(command.label(), &self.query))
            .collect()
    }
}

fn visible_start(selected: usize, count: usize, available: usize) -> usize {
    selected
        .saturating_sub(available.saturating_sub(1))
        .min(count.saturating_sub(available))
}

fn fuzzy_match(candidate: &str, query: &str) -> bool {
    let mut candidate = candidate.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|needle| candidate.by_ref().any(|character| character == needle))
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend, style::Color};

    use oxyst_config::CommandId;

    use super::{CommandPalette, fuzzy_match, visible_start};

    #[test]
    fn fuzzy_search_matches_subsequences() {
        assert!(fuzzy_match("Export as PDF", "epdf"));
        assert!(!fuzzy_match("Export as SVG", "pdf"));

        let mut palette = CommandPalette::default();
        palette.input_text("png");
        assert_eq!(palette.selected(), Some(CommandId::ExportPng));
    }

    #[test]
    fn result_window_tracks_the_selection() {
        let command_count = CommandId::all()
            .filter(|command| command.show_in_palette())
            .count();
        assert_eq!(visible_start(0, command_count, 2), 0);
        assert_eq!(visible_start(5, command_count, 2), 4);
        assert_eq!(
            visible_start(command_count - 1, command_count, 2),
            command_count - 2
        );
        assert_eq!(visible_start(0, command_count, 0), 0);
    }

    #[test]
    fn draws_selected_results_in_short_terminals() -> Result<(), Infallible> {
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);
        let mut palette = CommandPalette::default();
        let command_count = CommandId::all()
            .filter(|command| command.show_in_palette())
            .count();
        for _ in 1..command_count {
            palette.move_selection(1);
        }
        let mut terminal = Terminal::new(TestBackend::new(40, 8))?;
        terminal.draw(|frame| palette.draw(frame, &theme))?;
        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Quit"));
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.symbol() == "Q" && cell.bg == Color::DarkGray)
        );

        let mut filtered = CommandPalette::default();
        filtered.input_text("export");
        filtered.move_selection(2);
        terminal.draw(|frame| filtered.draw(frame, &theme))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Export as SVG"));

        let mut tiny = Terminal::new(TestBackend::new(10, 2))?;
        tiny.draw(|frame| filtered.draw(frame, &theme))?;
        Ok(())
    }
}
