use std::{cmp, ops::Range};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use typst_syntax::{LinkedNode, Source, Tag, highlight};
use typst_tui_compiler::Severity;
use typst_tui_document::Document;

use crate::action::Action;

use super::Diagnostics;

#[derive(Debug)]
pub(crate) struct Editor {
    vertical_scroll: usize,
    horizontal_scroll: usize,
    source: Source,
    highlighted_lines: Vec<Line<'static>>,
}

impl Editor {
    pub(crate) fn new(text: &str) -> Self {
        let source = Source::detached(text);
        let highlighted_lines = highlighted_lines(&source);
        Self {
            vertical_scroll: 0,
            horizontal_scroll: 0,
            source,
            highlighted_lines,
        }
    }

    pub(crate) fn update(&mut self, action: &Action, document: &mut Document) {
        let revision = document.revision();
        match action {
            Action::Insert(character) => document.insert_char(*character),
            Action::InsertText(text) => document.insert_text(text),
            Action::Backspace => document.backspace(),
            Action::Delete => document.delete(),
            Action::Move(motion) => document.move_cursor(*motion),
            Action::Undo => document.undo(),
            Action::Redo => document.redo(),
            Action::Recompile
            | Action::CompileFinished(_)
            | Action::Tick
            | Action::SwitchFocus
            | Action::ToggleDiagnostics
            | Action::NavigateDiagnostic(_)
            | Action::Click { .. }
            | Action::ScrollAt { .. }
            | Action::ScrollPreviewPages(_)
            | Action::Save
            | Action::RequestQuit
            | Action::Quit
            | Action::CancelQuit => {}
        }

        if document.revision() != revision {
            if let Some(edit) = document.last_edit() {
                self.source.edit(edit.range(), edit.replacement());
            } else {
                self.source.replace(&document.text());
            }
            self.highlighted_lines = highlighted_lines(&self.source);
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

        let number_width = document.line_count().to_string().len() as u16;
        let gutter_width = number_width + 2;
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
                let (marker, color) = match diagnostics.severity_at(line) {
                    Some(Severity::Error) => ("E", Color::Red),
                    Some(Severity::Warning) => ("W", Color::Yellow),
                    None => (" ", Color::Reset),
                };
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(color)),
                    Span::styled(
                        format!("{:>width$} ", line + 1, width = usize::from(number_width)),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let text = (self.vertical_scroll..end_line)
            .map(|line| {
                let style = if line == cursor.line {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };
                self.highlighted_lines.get(line).cloned().map_or_else(
                    || Line::from(document.line(line).unwrap_or_default()).style(style),
                    |content| content.style(style),
                )
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

impl Default for Editor {
    fn default() -> Self {
        Self::new("")
    }
}

#[derive(Debug)]
struct StyledRange {
    range: Range<usize>,
    style: Style,
}

fn highlighted_lines(source: &Source) -> Vec<Line<'static>> {
    let mut ranges = Vec::new();
    collect_ranges(
        &LinkedNode::new(source.root()),
        Style::default(),
        &mut ranges,
    );

    let mut first_range = 0;
    (0..source.lines().len_lines())
        .filter_map(|line| source.lines().line_to_range(line))
        .map(|range| highlighted_line(source.text(), range, &ranges, &mut first_range))
        .collect()
}

fn collect_ranges(node: &LinkedNode<'_>, inherited: Style, ranges: &mut Vec<StyledRange>) {
    let style = highlight(node).map_or(inherited, |tag| inherited.patch(style_for(tag)));
    if !node.leaf_text().is_empty() {
        ranges.push(StyledRange {
            range: node.range(),
            style,
        });
        return;
    }

    for child in node.children() {
        collect_ranges(&child, style, ranges);
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
            spans.push(Span::styled(text[start..end].to_owned(), range.style));
            cursor = end;
        }
    }
    if cursor < content_range.end {
        spans.push(Span::raw(text[cursor..content_range.end].to_owned()));
    }

    Line::from(spans)
}

fn style_for(tag: Tag) -> Style {
    match tag {
        Tag::Comment => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::ITALIC),
        Tag::Punctuation => Style::default().fg(Color::Gray),
        Tag::Escape | Tag::MathDelimiter | Tag::MathOperator | Tag::MathGroupingParens => {
            Style::default().fg(Color::LightMagenta)
        }
        Tag::Strong => Style::default().add_modifier(Modifier::BOLD),
        Tag::Emph => Style::default().add_modifier(Modifier::ITALIC),
        Tag::Link => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::UNDERLINED),
        Tag::Raw | Tag::String => Style::default().fg(Color::Green),
        Tag::Label | Tag::Ref => Style::default().fg(Color::LightCyan),
        Tag::Heading | Tag::ListMarker | Tag::ListTerm => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        Tag::Keyword => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        Tag::Operator => Style::default().fg(Color::LightMagenta),
        Tag::Number => Style::default().fg(Color::LightBlue),
        Tag::Function => Style::default().fg(Color::LightBlue),
        Tag::Interpolated => Style::default().fg(Color::Cyan),
        Tag::Error => Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::UNDERLINED),
    }
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

    use super::{Diagnostics, Editor};
    use crate::action::Action;

    #[test]
    fn draws_line_numbers_and_buffer_text() -> Result<(), Infallible> {
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;
        let document = Document::new("first\nsecond");
        let mut editor = Editor::new("first\nsecond");
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
        let mut editor = Editor::new(source);
        let diagnostics = Diagnostics::default();
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend)?;

        terminal.draw(|frame| {
            editor.draw(frame, frame.area(), &document, &diagnostics, true);
        })?;

        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| cell.fg == Color::Yellow));
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.fg == Color::LightBlue)
        );

        Ok(())
    }

    #[test]
    fn applies_document_edits_to_the_syntax_source() {
        let source = "#let value = 1";
        let mut document = Document::new(source);
        document.move_cursor(Motion::DocumentEnd);
        let mut editor = Editor::new(source);

        editor.update(&Action::Insert('0'), &mut document);
        assert_eq!(editor.source.text(), "#let value = 10");

        editor.update(&Action::Undo, &mut document);
        assert_eq!(editor.source.text(), source);
    }

    #[test]
    fn draws_main_source_diagnostic_markers() -> Result<(), Infallible> {
        let document = Document::new("first\nsecond");
        let mut editor = Editor::new("first\nsecond");
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
}
