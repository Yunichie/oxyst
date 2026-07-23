use oxyst_theme::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::Style,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::style::{base, color};

use super::modal_area;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchMode {
    Find,
    Replace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Field {
    Query,
    Replacement,
}

#[derive(Debug)]
pub(crate) struct Search {
    mode: SearchMode,
    field: Field,
    query: String,
    replacement: String,
}

impl Search {
    pub(crate) fn new(mode: SearchMode, query: impl Into<String>) -> Self {
        Self {
            mode,
            field: Field::Query,
            query: query.into(),
            replacement: String::new(),
        }
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn replacement(&self) -> &str {
        &self.replacement
    }

    pub(crate) fn can_replace(&self) -> bool {
        self.mode == SearchMode::Replace
    }

    pub(crate) fn input(&mut self, character: char) {
        self.active_mut().push(character);
    }

    pub(crate) fn input_text(&mut self, text: &str) {
        self.active_mut().push_str(text);
    }

    pub(crate) fn backspace(&mut self) {
        self.active_mut().pop();
    }

    pub(crate) fn toggle_field(&mut self) {
        if self.mode == SearchMode::Replace {
            self.field = match self.field {
                Field::Query => Field::Replacement,
                Field::Replacement => Field::Query,
            };
        }
    }

    pub(crate) fn query_is_active(&self) -> bool {
        self.field == Field::Query
    }

    pub(crate) fn draw(&self, frame: &mut Frame, theme: &Theme) {
        let height = if self.mode == SearchMode::Replace {
            7
        } else {
            5
        };
        let area = modal_area(frame.area(), 72, height);
        frame.render_widget(Clear, area);
        let title = match self.mode {
            SearchMode::Find => " Find ",
            SearchMode::Replace => " Find and replace ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(theme.accent)))
            .style(base(theme))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let rows = if self.mode == SearchMode::Replace {
            Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Fill(1),
            ])
            .split(inner)
        } else {
            Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).split(inner)
        };
        draw_field(
            frame,
            rows[0],
            "Find",
            &self.query,
            self.field == Field::Query,
            theme,
        );
        if self.mode == SearchMode::Replace {
            draw_field(
                frame,
                rows[1],
                "Replace",
                &self.replacement,
                self.field == Field::Replacement,
                theme,
            );
        }
    }

    fn active_mut(&mut self) -> &mut String {
        match self.field {
            Field::Query => &mut self.query,
            Field::Replacement => &mut self.replacement,
        }
    }
}

fn draw_field(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    label: &str,
    value: &str,
    active: bool,
    theme: &Theme,
) {
    let label_width = 9_u16;
    let [label_area, input_area] =
        Layout::horizontal([Constraint::Length(label_width), Constraint::Fill(1)]).areas(area);
    let cursor = u16::try_from(UnicodeWidthStr::width(value)).unwrap_or(u16::MAX);
    let visible_width = input_area.width.max(1);
    let scroll = cursor.saturating_sub(visible_width.saturating_sub(1));
    let label_style = Style::default().fg(color(if active { theme.accent } else { theme.muted }));
    frame.render_widget(
        Paragraph::new(format!("{label:<8}")).style(label_style),
        label_area,
    );
    frame.render_widget(Paragraph::new(value).scroll((0, scroll)), input_area);
    if active && input_area.width > 0 {
        frame.set_cursor_position((
            input_area
                .x
                .saturating_add(cursor.saturating_sub(scroll))
                .min(input_area.right().saturating_sub(1)),
            input_area.y,
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{Search, SearchMode};

    #[test]
    fn draws_and_edits_both_replace_fields() -> Result<(), Infallible> {
        let mut search = Search::new(SearchMode::Replace, "old");
        search.toggle_field();
        search.input_text("new");
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend)?;
        let theme = Theme::new(ThemeName::Dark, ColorDepth::Ansi16);

        terminal.draw(|frame| search.draw(frame, &theme))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("old"));
        assert!(rendered.contains("new"));
        assert_eq!(search.replacement(), "new");
        Ok(())
    }
}
