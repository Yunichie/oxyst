use std::{cmp, ops::Range};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use typst_syntax::{LinkedNode, Source, highlight};
use typst_tui_compiler::Severity;
use typst_tui_document::Document;
use typst_tui_theme::{TextStyle, Theme};

use crate::{
    action::Action,
    style::{color, text_style},
};

use super::Diagnostics;

#[derive(Debug)]
pub(crate) struct Editor {
    vertical_scroll: usize,
    horizontal_scroll: usize,
    inner: Rect,
    text_area: Rect,
    source: Source,
    highlighted_lines: Vec<Line<'static>>,
    theme: Theme,
}

impl Editor {
    pub(crate) fn new(text: &str, theme: Theme) -> Self {
        let source = Source::detached(text);
        let highlighted_lines = highlighted_lines(&source, theme);
        Self {
            vertical_scroll: 0,
            horizontal_scroll: 0,
            inner: Rect::default(),
            text_area: Rect::default(),
            source,
            highlighted_lines,
            theme,
        }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.highlighted_lines = highlighted_lines(&self.source, theme);
    }

    pub(crate) fn update(&mut self, action: &Action, document: &mut Document) {
        let revision = document.revision();
        match action {
            Action::Insert(character) => document.insert_char(*character),
            Action::InsertText(text) => document.insert_text(text),
            Action::Backspace => document.backspace(),
            Action::Delete => document.delete(),
            Action::Move(motion) => document.move_cursor(*motion),
            Action::Select(motion) => document.move_cursor_selecting(*motion, true),
            Action::SelectAll => document.select_all(),
            Action::Undo => document.undo(),
            Action::Redo => document.redo(),
            _ => {}
        }

        if document.revision() != revision {
            if let Some(edit) = document.last_edit() {
                self.source.edit(edit.range(), edit.replacement());
            } else {
                self.source.replace(&document.text());
            }
            self.highlighted_lines = highlighted_lines(&self.source, self.theme);
        }
    }

    pub(crate) fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        document: &Document,
        diagnostics: &Diagnostics,
        focused: bool,
    ) {
        let border_color = if focused {
            self.theme.accent
        } else {
            self.theme.border
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(border_color)))
            .title(" Editor ");
        let inner = block.inner(area);
        self.inner = inner;
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let number_width = document.line_count().to_string().len() as u16;
        let gutter_width = number_width + 2;
        let [gutter_area, text_area] = Layout::horizontal([
            Constraint::Length(gutter_width.min(inner.width)),
            Constraint::Fill(1),
        ])
        .areas(inner);
        self.text_area = text_area;
        let cursor = document.cursor_position();
        self.keep_cursor_visible(cursor.line, cursor.visual_column, text_area);

        let visible_lines = usize::from(text_area.height);
        let end_line = (self.vertical_scroll + visible_lines).min(document.line_count());
        let gutter = (self.vertical_scroll..end_line)
            .map(|line| {
                let (marker, marker_color) = match diagnostics.severity_at(line) {
                    Some(Severity::Error) => ("E", self.theme.error),
                    Some(Severity::Warning) => ("W", self.theme.warning),
                    None => (" ", typst_tui_theme::Color::Reset),
                };
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(color(marker_color))),
                    Span::styled(
                        format!("{:>width$} ", line + 1, width = usize::from(number_width)),
                        Style::default().fg(color(self.theme.muted)),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let selection = document.selection_byte_range();
        let text = (self.vertical_scroll..end_line)
            .map(|line| {
                let style = if line == cursor.line {
                    Style::default().bg(color(self.theme.current_line))
                } else {
                    Style::default()
                };
                let content = self.highlighted_lines.get(line).cloned().map_or_else(
                    || Line::from(document.line(line).unwrap_or_default()).style(style),
                    |content| content.style(style),
                );
                let line_start = self
                    .source
                    .lines()
                    .line_to_range(line)
                    .map_or(0, |range| range.start);
                apply_selection(content, line_start, selection.as_ref(), self.theme)
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

    pub(crate) fn contains(&self, column: u16, row: u16) -> bool {
        column >= self.inner.x
            && column < self.inner.right()
            && row >= self.inner.y
            && row < self.inner.bottom()
    }

    pub(crate) fn hide(&mut self) {
        self.inner = Rect::default();
        self.text_area = Rect::default();
    }

    pub(crate) fn place_cursor(
        &self,
        document: &mut Document,
        column: u16,
        row: u16,
        selecting: bool,
    ) -> bool {
        if !self.contains(column, row) {
            return false;
        }
        let line = self
            .vertical_scroll
            .saturating_add(usize::from(row.saturating_sub(self.text_area.y)))
            .min(document.line_count().saturating_sub(1));
        let visual_column = if column < self.text_area.x {
            0
        } else {
            self.horizontal_scroll
                .saturating_add(usize::from(column - self.text_area.x))
        };
        document.set_cursor_visual_position(line, visual_column, selecting)
    }

    pub(crate) fn scroll_lines(&mut self, lines: isize, document: &Document) {
        self.vertical_scroll = self
            .vertical_scroll
            .saturating_add_signed(lines)
            .min(document.line_count().saturating_sub(1));
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(
            "",
            Theme::new(
                typst_tui_theme::ThemeName::Dark,
                typst_tui_theme::ColorDepth::Ansi16,
            ),
        )
    }
}

#[derive(Debug)]
struct StyledRange {
    range: Range<usize>,
    style: TextStyle,
}

fn highlighted_lines(source: &Source, theme: Theme) -> Vec<Line<'static>> {
    let mut ranges = Vec::new();
    collect_ranges(
        &LinkedNode::new(source.root()),
        TextStyle::default(),
        theme,
        &mut ranges,
    );

    let mut first_range = 0;
    (0..source.lines().len_lines())
        .filter_map(|line| source.lines().line_to_range(line))
        .map(|range| highlighted_line(source.text(), range, &ranges, &mut first_range))
        .collect()
}

fn collect_ranges(
    node: &LinkedNode<'_>,
    inherited: TextStyle,
    theme: Theme,
    ranges: &mut Vec<StyledRange>,
) {
    let style = highlight(node).map_or(inherited, |tag| inherited.patch(theme.syntax(tag)));
    if !node.leaf_text().is_empty() {
        ranges.push(StyledRange {
            range: node.range(),
            style,
        });
        return;
    }

    for child in node.children() {
        collect_ranges(&child, style, theme, ranges);
    }
}

fn highlighted_line(
    text: &str,
    line_range: Range<usize>,
    ranges: &[StyledRange],
    first_range: &mut usize,
) -> Line<'static> {
    let line = &text[line_range.clone()];
    let content = line.trim_end_matches([
        '\r', '\n', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}',
    ]);
    let content_range = line_range.start..line_range.start + content.len();
    while ranges
        .get(*first_range)
        .is_some_and(|range| range.range.end <= content_range.start)
    {
        *first_range += 1;
    }

    let mut spans = Vec::new();
    let mut cursor = content_range.start;
    for range in ranges.iter().skip(*first_range) {
        if range.range.start >= content_range.end {
            break;
        }
        let start = cmp::max(range.range.start, content_range.start);
        let end = cmp::min(range.range.end, content_range.end);
        if cursor < start {
            spans.push(Span::raw(text[cursor..start].to_owned()));
        }
        if start < end {
            spans.push(Span::styled(
                text[start..end].to_owned(),
                text_style(range.style),
            ));
            cursor = end;
        }
    }
    if cursor < content_range.end {
        spans.push(Span::raw(text[cursor..content_range.end].to_owned()));
    }

    Line::from(spans)
}

fn apply_selection(
    line: Line<'static>,
    line_start: usize,
    selection: Option<&Range<usize>>,
    theme: Theme,
) -> Line<'static> {
    let Some(selection) = selection else {
        return line;
    };
    let mut offset = line_start;
    let mut spans = Vec::new();
    for span in line.spans {
        let content = span.content.into_owned();
        let end = offset + content.len();
        let selected_start = selection.start.clamp(offset, end);
        let selected_end = selection.end.clamp(offset, end);
        if offset < selected_start {
            spans.push(Span::styled(
                content[..selected_start - offset].to_owned(),
                span.style,
            ));
        }
        if selected_start < selected_end {
            spans.push(Span::styled(
                content[selected_start - offset..selected_end - offset].to_owned(),
                span.style.bg(color(theme.selection)),
            ));
        }
        if selected_end < end {
            spans.push(Span::styled(
                content[selected_end - offset..].to_owned(),
                span.style,
            ));
        }
        offset = end;
    }
    Line::from(spans).style(line.style)
}

fn scroll_as_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use ratatui::{Terminal, backend::TestBackend, style::Color};
    use typst_tui_compiler::{Diagnostic, Severity};
    use typst_tui_document::{Document, Motion};
    use typst_tui_theme::{ColorDepth, Theme, ThemeName};

    use super::{Diagnostics, Editor};
    use crate::action::Action;

    fn theme() -> Theme {
        Theme::new(ThemeName::Dark, ColorDepth::Ansi16)
    }

    #[test]
    fn draws_line_numbers_and_buffer_text() -> Result<(), Infallible> {
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;
        let document = Document::new("first\nsecond");
        let mut editor = Editor::new("first\nsecond", theme());
        let diagnostics = Diagnostics::default();

        terminal.draw(|frame| {
            editor.draw(frame, frame.area(), &document, &diagnostics, true);
        })?;

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

    #[test]
    fn applies_typst_syntax_styles() -> Result<(), Infallible> {
        let source = "= Heading\n#let answer = 42";
        let document = Document::new(source);
        let mut editor = Editor::new(source, theme());
        let diagnostics = Diagnostics::default();
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend)?;

        terminal.draw(|frame| {
            editor.draw(frame, frame.area(), &document, &diagnostics, true);
        })?;

        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| cell.fg == Color::Yellow));
        assert!(buffer.content().iter().any(|cell| cell.fg == Color::Blue));

        Ok(())
    }

    #[test]
    fn applies_document_edits_to_the_syntax_source() {
        let source = "#let value = 1";
        let mut document = Document::new(source);
        document.move_cursor(Motion::DocumentEnd);
        let mut editor = Editor::new(source, theme());

        editor.update(&Action::Insert('0'), &mut document);
        assert_eq!(editor.source.text(), "#let value = 10");

        editor.update(&Action::Undo, &mut document);
        assert_eq!(editor.source.text(), source);
    }

    #[test]
    fn draws_main_source_diagnostic_markers() -> Result<(), Infallible> {
        let document = Document::new("first\nsecond");
        let mut editor = Editor::new("first\nsecond", theme());
        let mut diagnostics = Diagnostics::default();
        diagnostics.set_items(vec![Diagnostic {
            severity: Severity::Error,
            message: "broken".to_owned(),
            path: Some("main.typ".to_owned()),
            line: Some(1),
            column: Some(0),
            is_main: true,
        }]);
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;

        terminal.draw(|frame| {
            editor.draw(frame, frame.area(), &document, &diagnostics, true);
        })?;

        let buffer = terminal.backend().buffer();
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.symbol() == "E" && cell.fg == Color::Red)
        );

        Ok(())
    }

    #[test]
    fn selection_background_preserves_the_editor_content() -> Result<(), Infallible> {
        let mut document = Document::new("first");
        document.move_cursor_selecting(Motion::Right, true);
        document.move_cursor_selecting(Motion::Right, true);
        let mut editor = Editor::new("first", theme());
        let diagnostics = Diagnostics::default();
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend)?;

        terminal.draw(|frame| editor.draw(frame, frame.area(), &document, &diagnostics, true))?;

        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| { matches!(cell.symbol(), "f" | "i") && cell.bg == Color::DarkGray })
        );
        Ok(())
    }

    #[test]
    fn mouse_position_uses_visual_columns_for_wide_text() -> Result<(), Infallible> {
        let mut document = Document::new("one\n界two");
        let mut editor = Editor::new("one\n界two", theme());
        let diagnostics = Diagnostics::default();
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), &document, &diagnostics, true))?;

        assert!(editor.place_cursor(&mut document, 6, 2, false));
        assert_eq!(document.cursor_position().line, 1);
        assert_eq!(document.cursor_position().visual_column, 2);
        Ok(())
    }
}
