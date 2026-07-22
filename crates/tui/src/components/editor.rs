use std::{cmp, collections::BTreeMap, ops::Range};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use typst_syntax::{LinkedNode, Source, Tag, highlight};
use typst_tui_compiler::Severity;
use typst_tui_document::{Document, Motion};
use typst_tui_theme::{TextStyle, Theme};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    action::Action,
    style::{color, text_style},
};

use super::{Component, Diagnostics};

#[derive(Debug)]
pub(crate) struct Editor {
    document: Document,
    vertical_scroll: usize,
    inner: Rect,
    text_area: Rect,
    source: Source,
    highlighted_lines: Vec<Line<'static>>,
    non_code_ranges: Vec<Range<usize>>,
    visual_rows: Vec<VisualRow>,
    layout_width: u16,
    preferred_visual_column: Option<usize>,
    diagnostic_lines: BTreeMap<usize, Severity>,
    theme: Theme,
}

impl Editor {
    pub(crate) fn new(text: &str, theme: Theme) -> Self {
        let source = Source::detached(text);
        let highlighted_lines = highlighted_lines(&source, theme);
        let non_code_ranges = non_code_ranges(&source);
        Self {
            document: Document::new(text),
            vertical_scroll: 0,
            inner: Rect::default(),
            text_area: Rect::default(),
            source,
            highlighted_lines,
            non_code_ranges,
            visual_rows: Vec::new(),
            layout_width: 0,
            preferred_visual_column: None,
            diagnostic_lines: BTreeMap::new(),
            theme,
        }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.highlighted_lines = highlighted_lines(&self.source, theme);
        self.layout_width = 0;
    }

    fn apply_action(&mut self, action: &Action) {
        let revision = self.document.revision();
        if !matches!(
            action,
            Action::Move(Motion::Up | Motion::Down) | Action::Select(Motion::Up | Motion::Down)
        ) {
            self.preferred_visual_column = None;
        }
        match action {
            Action::Insert(character) => self.document.insert_char(*character),
            Action::InsertText(text) => self.document.insert_text(text),
            Action::Backspace => self.document.backspace(),
            Action::Delete => self.document.delete(),
            Action::Move(Motion::Up) => self.move_vertically(-1, false),
            Action::Move(Motion::Down) => self.move_vertically(1, false),
            Action::Move(motion) => self.document.move_cursor(*motion),
            Action::Select(Motion::Up) => self.move_vertically(-1, true),
            Action::Select(Motion::Down) => self.move_vertically(1, true),
            Action::Select(motion) => self.document.move_cursor_selecting(*motion, true),
            Action::SelectAll => self.document.select_all(),
            Action::Undo => self.document.undo(),
            Action::Redo => self.document.redo(),
            _ => {}
        }

        if self.document.revision() != revision {
            if let Some(edit) = self.document.last_edit() {
                self.source.edit(edit.range(), edit.replacement());
            } else {
                self.source.replace(&self.document.text());
            }
            self.highlighted_lines = highlighted_lines(&self.source, self.theme);
            self.non_code_ranges = non_code_ranges(&self.source);
            self.layout_width = 0;
        }
    }

    fn draw_editor(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
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

        let number_width = self.document.line_count().to_string().len() as u16;
        let gutter_width = number_width + 2;
        let [gutter_area, text_area] = Layout::horizontal([
            Constraint::Length(gutter_width.min(inner.width)),
            Constraint::Fill(1),
        ])
        .areas(inner);
        self.text_area = text_area;
        let cursor = self.document.cursor_position();
        self.ensure_layout(text_area.width);
        let cursor_row = self.cursor_visual_row(cursor.line, cursor.visual_column);
        self.keep_cursor_visible(cursor_row, text_area);

        let visible_lines = usize::from(text_area.height);
        let end_line = (self.vertical_scroll + visible_lines).min(self.visual_rows.len());
        let gutter = (self.vertical_scroll..end_line)
            .map(|row| {
                let visual = &self.visual_rows[row];
                let (marker, marker_color) = if visual.continuation {
                    ("↪", self.theme.muted)
                } else {
                    match self.diagnostic_lines.get(&visual.logical_line).copied() {
                        Some(Severity::Error) => ("E", self.theme.error),
                        Some(Severity::Warning) => ("W", self.theme.warning),
                        None => (" ", typst_tui_theme::Color::Reset),
                    }
                };
                let number = if visual.continuation {
                    String::new()
                } else {
                    (visual.logical_line + 1).to_string()
                };
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(color(marker_color))),
                    Span::styled(
                        format!("{number:>width$} ", width = usize::from(number_width)),
                        Style::default().fg(color(self.theme.muted)),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let selection = self.document.selection_byte_range();
        let brackets = matching_brackets(
            self.source.text(),
            self.document.cursor_byte_index(),
            &self.non_code_ranges,
        );
        let text = (self.vertical_scroll..end_line)
            .map(|row| {
                let visual = &self.visual_rows[row];
                let style = if visual.logical_line == cursor.line {
                    Style::default().bg(color(self.theme.current_line))
                } else {
                    Style::default()
                };
                let content = apply_bracket_matches(
                    visual.content.clone().style(style),
                    visual.byte_start,
                    brackets.as_ref(),
                );
                apply_selection(content, visual.byte_start, selection.as_ref(), self.theme)
            })
            .collect::<Vec<_>>();

        frame.render_widget(Paragraph::new(gutter), gutter_area);
        frame.render_widget(Paragraph::new(text), text_area);

        if focused && cursor_row >= self.vertical_scroll && cursor_row < end_line {
            let row = &self.visual_rows[cursor_row];
            let cursor_x = cursor
                .visual_column
                .saturating_sub(row.start_visual_column)
                .min(usize::from(text_area.width.saturating_sub(1)));
            let x = text_area.x.saturating_add(scroll_as_u16(cursor_x));
            let y = text_area
                .y
                .saturating_add(scroll_as_u16(cursor_row - self.vertical_scroll));
            if x < text_area.right() && y < text_area.bottom() {
                frame.set_cursor_position((x, y));
            }
        }
    }

    fn keep_cursor_visible(&mut self, row: usize, area: Rect) {
        let height = usize::from(area.height.max(1));
        if row < self.vertical_scroll {
            self.vertical_scroll = row;
        } else if row >= self.vertical_scroll + height {
            self.vertical_scroll = row + 1 - height;
        }
        self.vertical_scroll = self
            .vertical_scroll
            .min(self.visual_rows.len().saturating_sub(1));
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

    pub(crate) fn place_cursor(&mut self, column: u16, row: u16, selecting: bool) -> bool {
        if !self.contains(column, row) {
            return false;
        }
        self.preferred_visual_column = None;
        let row = self
            .vertical_scroll
            .saturating_add(usize::from(row.saturating_sub(self.text_area.y)))
            .min(self.visual_rows.len().saturating_sub(1));
        let visual = &self.visual_rows[row];
        let visual_column = if column < self.text_area.x {
            visual.start_visual_column
        } else {
            visual
                .start_visual_column
                .saturating_add(usize::from(column - self.text_area.x))
        };
        self.document
            .set_cursor_visual_position(visual.logical_line, visual_column, selecting)
    }

    pub(crate) fn scroll_lines(&mut self, lines: isize) {
        self.vertical_scroll = self
            .vertical_scroll
            .saturating_add_signed(lines)
            .min(self.visual_rows.len().saturating_sub(1));
    }

    pub(crate) fn reset_preferred_visual_column(&mut self) {
        self.preferred_visual_column = None;
    }

    fn ensure_layout(&mut self, width: u16) {
        let width = width.max(1);
        if self.layout_width == width && !self.visual_rows.is_empty() {
            return;
        }
        self.layout_width = width;
        self.preferred_visual_column = None;
        self.visual_rows.clear();
        for (logical_line, content) in self.highlighted_lines.iter().enumerate() {
            let byte_start = self
                .source
                .lines()
                .line_to_range(logical_line)
                .map_or(0, |range| range.start);
            self.visual_rows.extend(wrap_line(
                content,
                logical_line,
                byte_start,
                usize::from(width),
            ));
        }
        if self.visual_rows.is_empty() {
            self.visual_rows.push(VisualRow::empty(0, 0, 0, false));
        }
        self.vertical_scroll = self
            .vertical_scroll
            .min(self.visual_rows.len().saturating_sub(1));
    }

    fn cursor_visual_row(&self, logical_line: usize, visual_column: usize) -> usize {
        self.visual_rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                row.logical_line == logical_line && row.start_visual_column <= visual_column
            })
            .map(|(index, _)| index)
            .next_back()
            .or_else(|| {
                self.visual_rows
                    .iter()
                    .position(|row| row.logical_line == logical_line)
            })
            .unwrap_or(0)
    }

    fn move_vertically(&mut self, direction: isize, selecting: bool) {
        let motion = if direction < 0 {
            Motion::Up
        } else {
            Motion::Down
        };
        if self.visual_rows.is_empty()
            || (!selecting && self.document.selection_byte_range().is_some())
        {
            self.preferred_visual_column = None;
            self.document.move_cursor_selecting(motion, selecting);
            return;
        }

        let cursor = self.document.cursor_position();
        let current = self.cursor_visual_row(cursor.line, cursor.visual_column);
        let target = current
            .saturating_add_signed(direction)
            .min(self.visual_rows.len().saturating_sub(1));
        let current_row = &self.visual_rows[current];
        let target_row = &self.visual_rows[target];
        let column = self.preferred_visual_column.unwrap_or_else(|| {
            cursor
                .visual_column
                .saturating_sub(current_row.start_visual_column)
        });
        if self.document.set_cursor_visual_position(
            target_row.logical_line,
            target_row.start_visual_column.saturating_add(column),
            selecting,
        ) {
            self.preferred_visual_column = Some(column);
        }
    }

    pub(crate) fn set_diagnostics(&mut self, diagnostics: &Diagnostics) {
        self.diagnostic_lines = diagnostics.line_severities().collect();
    }

    pub(crate) fn replace_document(&mut self, text: &str) {
        *self = Self::new(text, self.theme);
    }

    pub(crate) fn text(&self) -> String {
        self.document.text()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.document.revision()
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.document.is_dirty()
    }

    pub(crate) fn mark_saved(&mut self) {
        self.document.mark_saved();
    }

    pub(crate) fn last_edit(&self) -> Option<&typst_tui_document::TextEdit> {
        self.document.last_edit()
    }

    pub(crate) fn cursor_byte_index(&self) -> usize {
        self.document.cursor_byte_index()
    }

    pub(crate) fn cursor_position(&self) -> typst_tui_document::CursorPosition {
        self.document.cursor_position()
    }

    pub(crate) fn set_cursor_byte_index(&mut self, byte: usize) -> bool {
        let placed = self.document.set_cursor_byte_index(byte);
        if placed {
            self.preferred_visual_column = None;
        }
        placed
    }

    pub(crate) fn set_cursor_line_char(&mut self, line: usize, column: usize) -> bool {
        let placed = self.document.set_cursor_line_char(line, column);
        if placed {
            self.preferred_visual_column = None;
        }
        placed
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        self.document.selected_text()
    }

    pub(crate) fn selection_byte_range(&self) -> Option<Range<usize>> {
        self.document.selection_byte_range()
    }

    pub(crate) fn selection_graphemes(&self) -> usize {
        self.document.selection_graphemes()
    }

    pub(crate) fn word_count(&self) -> usize {
        self.document.word_count()
    }

    pub(crate) fn find(&self, query: &str, reverse: bool) -> Option<Range<usize>> {
        self.document.find(query, reverse)
    }

    pub(crate) fn select_byte_range(&mut self, range: Range<usize>) -> bool {
        let selected = self.document.select_byte_range(range);
        if selected {
            self.preferred_visual_column = None;
        }
        selected
    }
}

impl Component for Editor {
    fn update(&mut self, action: Action) {
        self.apply_action(&action);
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        self.draw_editor(frame, area, focused);
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

#[derive(Clone, Debug)]
struct VisualRow {
    logical_line: usize,
    start_visual_column: usize,
    byte_start: usize,
    content: Line<'static>,
    continuation: bool,
}

impl VisualRow {
    fn empty(
        logical_line: usize,
        start_visual_column: usize,
        byte_start: usize,
        continuation: bool,
    ) -> Self {
        Self {
            logical_line,
            start_visual_column,
            byte_start,
            content: Line::default(),
            continuation,
        }
    }
}

fn wrap_line(
    line: &Line<'static>,
    logical_line: usize,
    line_byte_start: usize,
    width: usize,
) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let mut spans = Vec::new();
    let mut row_width = 0_usize;
    let mut visual_column = 0_usize;
    let mut byte = line_byte_start;
    let mut row_byte_start = byte;
    let mut row_visual_start = visual_column;

    for span in &line.spans {
        for grapheme in span.content.graphemes(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if row_width > 0 && row_width.saturating_add(grapheme_width) > width {
                rows.push(VisualRow {
                    logical_line,
                    start_visual_column: row_visual_start,
                    byte_start: row_byte_start,
                    content: Line::from(std::mem::take(&mut spans)),
                    continuation: !rows.is_empty(),
                });
                row_width = 0;
                row_byte_start = byte;
                row_visual_start = visual_column;
            }
            push_span(&mut spans, grapheme, span.style);
            row_width = row_width.saturating_add(grapheme_width);
            visual_column = visual_column.saturating_add(grapheme_width);
            byte += grapheme.len();
        }
    }

    rows.push(VisualRow {
        logical_line,
        start_visual_column: row_visual_start,
        byte_start: row_byte_start,
        content: Line::from(spans),
        continuation: !rows.is_empty(),
    });
    rows
}

fn push_span(spans: &mut Vec<Span<'static>>, content: &str, style: Style) {
    if let Some(previous) = spans.last_mut()
        && previous.style == style
    {
        previous.content.to_mut().push_str(content);
        return;
    }
    spans.push(Span::styled(content.to_owned(), style));
}

fn non_code_ranges(source: &Source) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    collect_non_code_ranges(&LinkedNode::new(source.root()), &mut ranges);
    ranges
}

fn collect_non_code_ranges(node: &LinkedNode<'_>, ranges: &mut Vec<Range<usize>>) {
    if matches!(highlight(node), Some(Tag::Comment | Tag::Raw | Tag::String)) {
        ranges.push(node.range());
        return;
    }
    for child in node.children() {
        collect_non_code_ranges(&child, ranges);
    }
}

fn matching_brackets(
    text: &str,
    cursor: usize,
    non_code_ranges: &[Range<usize>],
) -> Option<[Range<usize>; 2]> {
    let candidate = bracket_at(text, cursor).or_else(|| {
        text.get(..cursor)
            .and_then(|before| before.char_indices().next_back())
            .filter(|(_, character)| is_bracket(*character))
    })?;
    let (start, bracket) = candidate;
    if is_non_code(start, non_code_ranges) {
        return None;
    }
    let (matching, forward) = match bracket {
        '(' => (')', true),
        '[' => (']', true),
        '{' => ('}', true),
        ')' => ('(', false),
        ']' => ('[', false),
        '}' => ('{', false),
        _ => return None,
    };
    let found = if forward {
        find_forward_match(text, start, bracket, matching, non_code_ranges)
    } else {
        find_backward_match(text, start, bracket, matching, non_code_ranges)
    }?;
    Some([
        start..start + bracket.len_utf8(),
        found..found + matching.len_utf8(),
    ])
}

fn bracket_at(text: &str, byte: usize) -> Option<(usize, char)> {
    let character = text.get(byte..)?.chars().next()?;
    is_bracket(character).then_some((byte, character))
}

fn is_bracket(character: char) -> bool {
    matches!(character, '(' | ')' | '[' | ']' | '{' | '}')
}

fn is_non_code(byte: usize, ranges: &[Range<usize>]) -> bool {
    ranges
        .partition_point(|range| range.start <= byte)
        .checked_sub(1)
        .is_some_and(|index| byte < ranges[index].end)
}

fn find_forward_match(
    text: &str,
    start: usize,
    opening: char,
    closing: char,
    non_code_ranges: &[Range<usize>],
) -> Option<usize> {
    let mut depth = 1_usize;
    for (offset, character) in text.get(start + opening.len_utf8()..)?.char_indices() {
        let byte = start + opening.len_utf8() + offset;
        if is_non_code(byte, non_code_ranges) {
            continue;
        }
        if character == opening {
            depth += 1;
        } else if character == closing {
            depth -= 1;
            if depth == 0 {
                return Some(byte);
            }
        }
    }
    None
}

fn find_backward_match(
    text: &str,
    start: usize,
    closing: char,
    opening: char,
    non_code_ranges: &[Range<usize>],
) -> Option<usize> {
    let mut depth = 1_usize;
    for (byte, character) in text.get(..start)?.char_indices().rev() {
        if is_non_code(byte, non_code_ranges) {
            continue;
        }
        if character == closing {
            depth += 1;
        } else if character == opening {
            depth -= 1;
            if depth == 0 {
                return Some(byte);
            }
        }
    }
    None
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

fn apply_bracket_matches(
    mut line: Line<'static>,
    line_start: usize,
    matches: Option<&[Range<usize>; 2]>,
) -> Line<'static> {
    let Some(matches) = matches else {
        return line;
    };
    for range in matches {
        line = apply_modifier(line, line_start, range, Modifier::REVERSED);
    }
    line
}

fn apply_modifier(
    line: Line<'static>,
    line_start: usize,
    range: &Range<usize>,
    modifier: Modifier,
) -> Line<'static> {
    let mut offset = line_start;
    let mut spans = Vec::new();
    for span in line.spans {
        let content = span.content.into_owned();
        let end = offset + content.len();
        let styled_start = range.start.clamp(offset, end);
        let styled_end = range.end.clamp(offset, end);
        if offset < styled_start {
            spans.push(Span::styled(
                content[..styled_start - offset].to_owned(),
                span.style,
            ));
        }
        if styled_start < styled_end {
            spans.push(Span::styled(
                content[styled_start - offset..styled_end - offset].to_owned(),
                span.style.add_modifier(modifier),
            ));
        }
        if styled_end < end {
            spans.push(Span::styled(
                content[styled_end - offset..].to_owned(),
                span.style,
            ));
        }
        offset = end;
    }
    Line::from(spans).style(line.style)
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
    use typst_syntax::Source;
    use typst_tui_compiler::{Diagnostic, Severity};
    use typst_tui_document::Motion;
    use typst_tui_theme::{ColorDepth, Theme, ThemeName};

    use super::{Component, Diagnostics, Editor, matching_brackets, non_code_ranges};
    use crate::action::Action;

    fn theme() -> Theme {
        Theme::new(ThemeName::Dark, ColorDepth::Ansi16)
    }

    #[test]
    fn draws_line_numbers_and_buffer_text() -> Result<(), Infallible> {
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;
        let mut editor = Editor::new("first\nsecond", theme());

        terminal.draw(|frame| {
            editor.draw(frame, frame.area(), true);
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
        let mut editor = Editor::new(source, theme());
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend)?;

        terminal.draw(|frame| {
            editor.draw(frame, frame.area(), true);
        })?;

        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| cell.fg == Color::Yellow));
        assert!(buffer.content().iter().any(|cell| cell.fg == Color::Blue));

        Ok(())
    }

    #[test]
    fn applies_document_edits_to_the_syntax_source() {
        let source = "#let value = 1";
        let mut editor = Editor::new(source, theme());
        editor.update(Action::Move(Motion::DocumentEnd));

        editor.update(Action::Insert('0'));
        assert_eq!(editor.source.text(), "#let value = 10");

        editor.update(Action::Undo);
        assert_eq!(editor.source.text(), source);
    }

    #[test]
    fn draws_main_source_diagnostic_markers() -> Result<(), Infallible> {
        let mut editor = Editor::new("first\nsecond", theme());
        let mut diagnostics = Diagnostics::default();
        diagnostics.set_items(vec![Diagnostic {
            severity: Severity::Error,
            message: "broken".to_owned(),
            path: Some("main.typ".to_owned()),
            line: Some(1),
            column: Some(0),
            is_main: true,
            notes: Vec::new(),
        }]);
        editor.set_diagnostics(&diagnostics);
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;

        terminal.draw(|frame| {
            editor.draw(frame, frame.area(), true);
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
        let mut editor = Editor::new("first", theme());
        editor.update(Action::Select(Motion::Right));
        editor.update(Action::Select(Motion::Right));
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend)?;

        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

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
        let mut editor = Editor::new("one\n界two", theme());
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

        assert!(editor.place_cursor(6, 2, false));
        assert_eq!(editor.cursor_position().line, 1);
        assert_eq!(editor.cursor_position().visual_column, 2);
        Ok(())
    }

    #[test]
    fn soft_wraps_wide_text_and_maps_continuation_clicks() -> Result<(), Infallible> {
        let source = "ab\u{754c}cdefgh";
        let mut editor = Editor::new(source, theme());
        let backend = TestBackend::new(12, 6);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

        assert!(editor.visual_rows.len() > 1);
        assert!(editor.visual_rows[1].continuation);
        let continuation_column = editor.visual_rows[1].start_visual_column;
        editor.update(Action::Move(Motion::Down));
        assert_eq!(editor.cursor_position().line, 0);
        assert_eq!(editor.cursor_position().visual_column, continuation_column);
        assert!(editor.place_cursor(editor.text_area.x, editor.text_area.y + 1, false,));
        assert_eq!(editor.cursor_position().visual_column, continuation_column);
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.symbol() == "↪")
        );
        Ok(())
    }

    #[test]
    fn vertical_motion_preserves_the_column_across_short_lines() -> Result<(), Infallible> {
        let source = "abcd\nx\nabcd";
        let mut editor = Editor::new(source, theme());
        let backend = TestBackend::new(30, 7);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert!(editor.set_cursor_line_char(0, 3));

        editor.update(Action::Move(Motion::Down));
        assert_eq!(editor.cursor_position().visual_column, 1);
        editor.update(Action::Move(Motion::Down));
        assert_eq!(editor.cursor_position().visual_column, 3);

        Ok(())
    }

    #[test]
    fn bracket_matching_ignores_strings() -> Result<(), &'static str> {
        let text = "#let value = (\"ignored )\" + [1])";
        let source = Source::detached(text);
        let non_code = non_code_ranges(&source);
        let opening = text.find('(').ok_or("opening bracket is missing")?;
        let closing = text.rfind(')').ok_or("closing bracket is missing")?;
        let matched =
            matching_brackets(text, opening, &non_code).ok_or("outer brackets did not match")?;
        assert_eq!(matched[1].start, closing);

        let string_bracket = text.find(")\"").ok_or("string bracket is missing")?;
        assert!(matching_brackets(text, string_bracket, &non_code).is_none());
        Ok(())
    }
}
