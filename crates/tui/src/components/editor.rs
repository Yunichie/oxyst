use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use typst_tui_document::Document;

use crate::action::Action;

#[derive(Debug, Default)]
pub(crate) struct Editor {
    vertical_scroll: usize,
    horizontal_scroll: usize,
}

impl Editor {
    pub(crate) fn update(&mut self, action: &Action, document: &mut Document) {
        match action {
            Action::Insert(character) => document.insert_char(*character),
            Action::InsertText(text) => document.insert_text(text),
            Action::Backspace => document.backspace(),
            Action::Delete => document.delete(),
            Action::Move(motion) => document.move_cursor(*motion),
            Action::Undo => document.undo(),
            Action::Redo => document.redo(),
            Action::Save | Action::RequestQuit | Action::Quit | Action::CancelQuit => {}
        }
    }

    pub(crate) fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        document: &Document,
        focused: bool,
    ) {
        let border_color = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(" Editor ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let gutter_width = document.line_count().to_string().len() as u16 + 1;
        let [gutter_area, text_area] = Layout::horizontal([
            Constraint::Length(gutter_width.min(inner.width)),
            Constraint::Fill(1),
        ])
        .areas(inner);
        let cursor = document.cursor_position();
        self.keep_cursor_visible(cursor.line, cursor.visual_column, text_area);

        let visible_lines = usize::from(text_area.height);
        let end_line = (self.vertical_scroll + visible_lines).min(document.line_count());
        let gutter = (self.vertical_scroll..end_line)
            .map(|line| {
                Line::from(Span::styled(
                    format!(
                        "{:>width$} ",
                        line + 1,
                        width = usize::from(gutter_width - 1)
                    ),
                    Style::default().fg(Color::DarkGray),
                ))
            })
            .collect::<Vec<_>>();
        let text = (self.vertical_scroll..end_line)
            .filter_map(|line| document.line(line).map(|content| (line, content)))
            .map(|(line, content)| {
                let style = if line == cursor.line {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };
                Line::from(content).style(style)
            })
            .collect::<Vec<_>>();

        frame.render_widget(Paragraph::new(gutter), gutter_area);
        frame.render_widget(
            Paragraph::new(text).scroll((0, scroll_as_u16(self.horizontal_scroll))),
            text_area,
        );

        if focused && cursor.line >= self.vertical_scroll && cursor.line < end_line {
            let cursor_x = cursor.visual_column.saturating_sub(self.horizontal_scroll);
            let x = text_area.x.saturating_add(scroll_as_u16(cursor_x));
            let y = text_area
                .y
                .saturating_add(scroll_as_u16(cursor.line - self.vertical_scroll));
            if x < text_area.right() && y < text_area.bottom() {
                frame.set_cursor_position((x, y));
            }
        }
    }

    fn keep_cursor_visible(&mut self, line: usize, visual_column: usize, area: Rect) {
        let height = usize::from(area.height.max(1));
        if line < self.vertical_scroll {
            self.vertical_scroll = line;
        } else if line >= self.vertical_scroll + height {
            self.vertical_scroll = line + 1 - height;
        }

        let width = usize::from(area.width.max(1));
        if visual_column < self.horizontal_scroll {
            self.horizontal_scroll = visual_column;
        } else if visual_column >= self.horizontal_scroll + width {
            self.horizontal_scroll = visual_column + 1 - width;
        }
    }
}

fn scroll_as_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use ratatui::{Terminal, backend::TestBackend};
    use typst_tui_document::Document;

    use super::Editor;

    #[test]
    fn draws_line_numbers_and_buffer_text() -> Result<(), Infallible> {
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;
        let mut editor = Editor::default();
        let document = Document::new("first\nsecond");

        terminal.draw(|frame| editor.draw(frame, frame.area(), &document, true))?;

        let buffer = terminal.backend().buffer();
        let rendered = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("1 first"));
        assert!(rendered.contains("2 second"));

        Ok(())
    }
}
