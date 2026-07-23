use std::{collections::BTreeMap, ops::Range};

use oxyst_compiler::Severity;
use oxyst_document::{Document, Motion, TextEdit};
use oxyst_theme::{TextStyle, Theme};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use typst_syntax::{LinkedNode, Side, Source, Tag, highlight};
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
    scroll: VisualPosition,
    follow_cursor: bool,
    inner: Rect,
    text_area: Rect,
    source: Source,
    line_rows: Vec<usize>,
    total_visual_rows: usize,
    viewport_rows: Vec<VisualRow>,
    layout_width: u16,
    preferred_visual_column: Option<usize>,
    diagnostic_lines: BTreeMap<usize, Severity>,
    theme: Theme,
}

impl Editor {
    pub(crate) fn new(text: &str, theme: Theme) -> Self {
        let source = Source::detached(text);
        let line_count = source.lines().len_lines();
        Self {
            document: Document::new(text),
            scroll: VisualPosition::default(),
            follow_cursor: true,
            inner: Rect::default(),
            text_area: Rect::default(),
            source,
            line_rows: vec![1; line_count],
            total_visual_rows: line_count,
            viewport_rows: Vec::new(),
            layout_width: 0,
            preferred_visual_column: None,
            diagnostic_lines: BTreeMap::new(),
            theme,
        }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    fn apply_action(&mut self, action: &Action) {
        let revision = self.document.revision();
        let cursor = self.document.cursor_char_index();
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
            if let Some(edit) = self.document.last_edit().cloned() {
                self.apply_source_edit(&edit);
            } else {
                self.source.replace(&self.document.text());
                self.reset_layout();
            }
        }
        if self.document.revision() != revision || self.document.cursor_char_index() != cursor {
            self.follow_cursor = true;
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
        let cursor_row = self.visual_position(cursor.line, cursor.visual_column);
        if self.follow_cursor {
            self.keep_cursor_visible(cursor_row, text_area);
        } else {
            self.clamp_scroll(text_area.height);
        }

        self.viewport_rows = self.build_viewport_rows(usize::from(text_area.height));
        let gutter = self
            .viewport_rows
            .iter()
            .map(|visual| {
                let (marker, marker_color) = if visual.continuation {
                    ("↪", self.theme.muted)
                } else {
                    match self.diagnostic_lines.get(&visual.logical_line).copied() {
                        Some(Severity::Error) => ("E", self.theme.error),
                        Some(Severity::Warning) => ("W", self.theme.warning),
                        None => (" ", oxyst_theme::Color::Reset),
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
        let brackets = matching_brackets(&self.source, self.document.cursor_byte_index());
        let text = self
            .viewport_rows
            .iter()
            .map(|visual| {
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

        if focused
            && let Some((screen_row, row)) = self
                .viewport_rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.position() == cursor_row)
        {
            let cursor_x = cursor
                .visual_column
                .saturating_sub(row.start_visual_column)
                .min(usize::from(text_area.width.saturating_sub(1)));
            let x = text_area.x.saturating_add(scroll_as_u16(cursor_x));
            let y = text_area.y.saturating_add(scroll_as_u16(screen_row));
            if x < text_area.right() && y < text_area.bottom() {
                frame.set_cursor_position((x, y));
            }
        }
    }

    fn keep_cursor_visible(&mut self, cursor: VisualPosition, area: Rect) {
        let height = usize::from(area.height.max(1));
        if cursor < self.scroll {
            self.scroll = cursor;
        } else if self.visual_distance(self.scroll, cursor) >= height {
            self.scroll = self.advance_visual_position(
                cursor,
                -isize::try_from(height.saturating_sub(1)).unwrap_or(isize::MAX),
            );
        }
        self.clamp_scroll(area.height);
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
        let screen_row = usize::from(row.saturating_sub(self.text_area.y));
        let Some(visual) = self
            .viewport_rows
            .get(screen_row)
            .or_else(|| self.viewport_rows.last())
        else {
            return false;
        };
        let visual_column = if column < self.text_area.x {
            visual.start_visual_column
        } else {
            visual
                .start_visual_column
                .saturating_add(usize::from(column - self.text_area.x))
        };
        let placed =
            self.document
                .set_cursor_visual_position(visual.logical_line, visual_column, selecting);
        if placed {
            self.follow_cursor = true;
        }
        placed
    }

    pub(crate) fn scroll_lines(&mut self, lines: isize) {
        self.follow_cursor = false;
        self.scroll = self.advance_visual_position(self.scroll, lines);
        self.clamp_scroll(self.text_area.height);
    }

    pub(crate) fn reset_preferred_visual_column(&mut self) {
        self.preferred_visual_column = None;
    }

    fn ensure_layout(&mut self, width: u16) {
        let width = width.max(1);
        if self.layout_width == width && self.line_rows.len() == self.source.lines().len_lines() {
            return;
        }
        self.layout_width = width;
        self.preferred_visual_column = None;
        self.line_rows = (0..self.source.lines().len_lines())
            .map(|line| line_row_count(&self.source, line, usize::from(width)))
            .collect();
        self.total_visual_rows = self.line_rows.iter().sum();
        self.clamp_scroll(self.text_area.height);
    }

    fn visual_position(&self, logical_line: usize, visual_column: usize) -> VisualPosition {
        let line = logical_line.min(self.line_rows.len().saturating_sub(1));
        let subrow = source_line_content(&self.source, line).map_or(0, |(_, content)| {
            visual_row_at_column(
                content,
                usize::from(self.layout_width.max(1)),
                visual_column,
            )
        });
        VisualPosition { line, subrow }
    }

    fn move_vertically(&mut self, direction: isize, selecting: bool) {
        let motion = if direction < 0 {
            Motion::Up
        } else {
            Motion::Down
        };
        if self.layout_width == 0 || (!selecting && self.document.selection_byte_range().is_some())
        {
            self.preferred_visual_column = None;
            self.document.move_cursor_selecting(motion, selecting);
            return;
        }

        let cursor = self.document.cursor_position();
        let current = self.visual_position(cursor.line, cursor.visual_column);
        let target = self.advance_visual_position(current, direction);
        let current_start = self.visual_row_start(current);
        let target_start = self.visual_row_start(target);
        let column = self
            .preferred_visual_column
            .unwrap_or_else(|| cursor.visual_column.saturating_sub(current_start));
        if self.document.set_cursor_visual_position(
            target.line,
            target_start.saturating_add(column),
            selecting,
        ) {
            self.preferred_visual_column = Some(column);
        }
    }

    fn apply_source_edit(&mut self, edit: &TextEdit) {
        let range = edit.range();
        let old_line_count = self.source.lines().len_lines();
        let start_line = self
            .source
            .lines()
            .byte_to_line(range.start)
            .unwrap_or(old_line_count.saturating_sub(1));
        let old_end_line = self
            .source
            .lines()
            .byte_to_line(range.end)
            .unwrap_or(old_line_count.saturating_sub(1));
        let metrics_match = self.line_rows.len() == old_line_count;

        self.source.edit(range.clone(), edit.replacement());

        if !metrics_match {
            self.reset_layout();
            return;
        }

        let new_line_count = self.source.lines().len_lines();
        let replacement_end = range
            .start
            .saturating_add(edit.replacement().len())
            .min(self.source.text().len());
        let new_end_line = self
            .source
            .lines()
            .byte_to_line(replacement_end)
            .unwrap_or(new_line_count.saturating_sub(1));
        let width = usize::from(self.layout_width.max(1));
        let replacement_rows = (start_line..=new_end_line)
            .map(|line| {
                if self.layout_width == 0 {
                    1
                } else {
                    line_row_count(&self.source, line, width)
                }
            })
            .collect::<Vec<_>>();
        let removed_rows: usize = self.line_rows[start_line..=old_end_line].iter().sum();
        let added_rows: usize = replacement_rows.iter().sum();
        self.line_rows
            .splice(start_line..=old_end_line, replacement_rows);
        self.total_visual_rows = self
            .total_visual_rows
            .saturating_sub(removed_rows)
            .saturating_add(added_rows);

        let old_span = old_end_line - start_line + 1;
        let new_span = new_end_line - start_line + 1;
        if self.scroll.line > old_end_line {
            self.scroll.line = self
                .scroll
                .line
                .saturating_sub(old_span)
                .saturating_add(new_span);
        } else if self.scroll.line >= start_line {
            self.scroll = VisualPosition {
                line: start_line,
                subrow: 0,
            };
        }
        self.viewport_rows.clear();
    }

    fn reset_layout(&mut self) {
        let line_count = self.source.lines().len_lines();
        self.line_rows = vec![1; line_count];
        self.total_visual_rows = line_count;
        self.viewport_rows.clear();
        self.layout_width = 0;
        self.scroll = VisualPosition::default();
    }

    fn build_viewport_rows(&self, height: usize) -> Vec<VisualRow> {
        let mut rows = Vec::with_capacity(height);
        let mut line = self.scroll.line;
        let mut skip = self.scroll.subrow;

        while rows.len() < height && line < self.line_rows.len() {
            rows.extend(wrap_line_window(
                &self.source,
                line,
                usize::from(self.layout_width.max(1)),
                skip,
                height - rows.len(),
            ));
            line += 1;
            skip = 0;
        }

        if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
            let ranges = styled_ranges(&self.source, first.byte_start..last.byte_end, self.theme);
            let mut first_range = 0;
            for row in &mut rows {
                row.content = highlighted_range(
                    self.source.text(),
                    row.byte_start..row.byte_end,
                    &ranges,
                    &mut first_range,
                );
            }
        }
        rows
    }

    fn visual_row_start(&self, position: VisualPosition) -> usize {
        source_line_content(&self.source, position.line).map_or(0, |(_, content)| {
            visual_column_at_row(
                content,
                usize::from(self.layout_width.max(1)),
                position.subrow,
            )
        })
    }

    fn advance_visual_position(
        &self,
        mut position: VisualPosition,
        distance: isize,
    ) -> VisualPosition {
        if self.line_rows.is_empty() {
            return VisualPosition::default();
        }
        position.line = position.line.min(self.line_rows.len() - 1);
        position.subrow = position
            .subrow
            .min(self.line_rows[position.line].saturating_sub(1));
        let mut remaining = distance.unsigned_abs();

        if distance >= 0 {
            while remaining > 0 {
                let available = self.line_rows[position.line]
                    .saturating_sub(position.subrow)
                    .saturating_sub(1);
                if remaining <= available {
                    position.subrow += remaining;
                    break;
                }
                if position.line + 1 == self.line_rows.len() {
                    position.subrow = self.line_rows[position.line].saturating_sub(1);
                    break;
                }
                remaining -= available + 1;
                position.line += 1;
                position.subrow = 0;
            }
        } else {
            while remaining > 0 {
                if remaining <= position.subrow {
                    position.subrow -= remaining;
                    break;
                }
                if position.line == 0 {
                    return VisualPosition::default();
                }
                remaining -= position.subrow + 1;
                position.line -= 1;
                position.subrow = self.line_rows[position.line].saturating_sub(1);
            }
        }

        position
    }

    fn visual_distance(&self, start: VisualPosition, end: VisualPosition) -> usize {
        if end < start {
            return self.visual_distance(end, start);
        }
        if start.line == end.line {
            return end.subrow.saturating_sub(start.subrow);
        }

        self.line_rows[start.line].saturating_sub(start.subrow)
            + self.line_rows[start.line + 1..end.line]
                .iter()
                .sum::<usize>()
            + end.subrow
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

    pub(crate) fn last_edit(&self) -> Option<&oxyst_document::TextEdit> {
        self.document.last_edit()
    }

    pub(crate) fn cursor_byte_index(&self) -> usize {
        self.document.cursor_byte_index()
    }

    pub(crate) fn cursor_position(&self) -> oxyst_document::CursorPosition {
        self.document.cursor_position()
    }

    pub(crate) fn set_cursor_byte_index(&mut self, byte: usize) -> bool {
        let placed = self.document.set_cursor_byte_index(byte);
        if placed {
            self.preferred_visual_column = None;
            self.follow_cursor = true;
        }
        placed
    }

    pub(crate) fn set_cursor_line_char(&mut self, line: usize, column: usize) -> bool {
        let placed = self.document.set_cursor_line_char(line, column);
        if placed {
            self.preferred_visual_column = None;
            self.follow_cursor = true;
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
            self.follow_cursor = true;
        }
        selected
    }

    fn max_scroll_position(&self, height: u16) -> VisualPosition {
        if self.line_rows.is_empty() || self.total_visual_rows <= usize::from(height.max(1)) {
            return VisualPosition::default();
        }
        let end = VisualPosition {
            line: self.line_rows.len() - 1,
            subrow: self.line_rows[self.line_rows.len() - 1].saturating_sub(1),
        };
        self.advance_visual_position(end, -isize::try_from(height.saturating_sub(1)).unwrap_or(0))
    }

    fn clamp_scroll(&mut self, height: u16) {
        if self.line_rows.is_empty() {
            self.scroll = VisualPosition::default();
            return;
        }
        self.scroll.line = self.scroll.line.min(self.line_rows.len() - 1);
        self.scroll.subrow = self
            .scroll
            .subrow
            .min(self.line_rows[self.scroll.line].saturating_sub(1));
        self.scroll = self.scroll.min(self.max_scroll_position(height));
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
                oxyst_theme::ThemeName::Dark,
                oxyst_theme::ColorDepth::Ansi16,
            ),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct VisualPosition {
    line: usize,
    subrow: usize,
}

#[derive(Debug)]
struct StyledRange {
    range: Range<usize>,
    style: Style,
}

#[derive(Clone, Debug)]
struct VisualRow {
    logical_line: usize,
    subrow: usize,
    start_visual_column: usize,
    byte_start: usize,
    byte_end: usize,
    content: Line<'static>,
    continuation: bool,
}

impl VisualRow {
    fn position(&self) -> VisualPosition {
        VisualPosition {
            line: self.logical_line,
            subrow: self.subrow,
        }
    }
}

fn source_line_content(source: &Source, line: usize) -> Option<(usize, &str)> {
    let range = source.lines().line_to_range(line)?;
    let content = source.text()[range.clone()].trim_end_matches([
        '\r', '\n', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}',
    ]);
    Some((range.start, content))
}

fn line_row_count(source: &Source, line: usize, width: usize) -> usize {
    source_line_content(source, line).map_or(1, |(_, content)| wrapped_row_count(content, width))
}

fn wrapped_row_count(content: &str, width: usize) -> usize {
    let mut rows = 1_usize;
    let mut row_width = 0_usize;
    for grapheme in content.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if row_width > 0 && row_width.saturating_add(grapheme_width) > width {
            rows += 1;
            row_width = 0;
        }
        row_width = row_width.saturating_add(grapheme_width);
    }
    rows
}

fn visual_row_at_column(content: &str, width: usize, target: usize) -> usize {
    let mut row = 0_usize;
    let mut row_width = 0_usize;
    let mut visual_column = 0_usize;
    for grapheme in content.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if row_width > 0 && row_width.saturating_add(grapheme_width) > width {
            row += 1;
            row_width = 0;
        }
        if visual_column >= target {
            return row;
        }
        row_width = row_width.saturating_add(grapheme_width);
        visual_column = visual_column.saturating_add(grapheme_width);
    }
    row
}

fn visual_column_at_row(content: &str, width: usize, target: usize) -> usize {
    if target == 0 {
        return 0;
    }
    let mut row = 0_usize;
    let mut row_width = 0_usize;
    let mut visual_column = 0_usize;
    for grapheme in content.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if row_width > 0 && row_width.saturating_add(grapheme_width) > width {
            row += 1;
            row_width = 0;
            if row == target {
                return visual_column;
            }
        }
        row_width = row_width.saturating_add(grapheme_width);
        visual_column = visual_column.saturating_add(grapheme_width);
    }
    visual_column
}

fn wrap_line_window(
    source: &Source,
    logical_line: usize,
    width: usize,
    skip: usize,
    limit: usize,
) -> Vec<VisualRow> {
    if limit == 0 {
        return Vec::new();
    }
    let Some((line_byte_start, content)) = source_line_content(source, logical_line) else {
        return Vec::new();
    };
    let mut rows = Vec::with_capacity(limit);
    let mut subrow = 0_usize;
    let mut row_width = 0_usize;
    let mut visual_column = 0_usize;
    let mut row_byte_start = 0_usize;
    let mut row_visual_start = 0_usize;

    for (byte, grapheme) in content.grapheme_indices(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if row_width > 0 && row_width.saturating_add(grapheme_width) > width {
            if subrow >= skip {
                rows.push(VisualRow {
                    logical_line,
                    subrow,
                    start_visual_column: row_visual_start,
                    byte_start: line_byte_start + row_byte_start,
                    byte_end: line_byte_start + byte,
                    content: Line::default(),
                    continuation: subrow > 0,
                });
                if rows.len() == limit {
                    return rows;
                }
            }
            subrow += 1;
            row_width = 0;
            row_byte_start = byte;
            row_visual_start = visual_column;
        }
        row_width = row_width.saturating_add(grapheme_width);
        visual_column = visual_column.saturating_add(grapheme_width);
    }

    if subrow >= skip {
        rows.push(VisualRow {
            logical_line,
            subrow,
            start_visual_column: row_visual_start,
            byte_start: line_byte_start + row_byte_start,
            byte_end: line_byte_start + content.len(),
            content: Line::default(),
            continuation: subrow > 0,
        });
    }
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

fn matching_brackets(source: &Source, cursor: usize) -> Option<[Range<usize>; 2]> {
    let text = source.text();
    let candidate = bracket_at(text, cursor).or_else(|| {
        text.get(..cursor)
            .and_then(|before| before.char_indices().next_back())
            .filter(|(_, character)| is_bracket(*character))
    })?;
    let (start, bracket) = candidate;
    if is_non_code(source, start) {
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
        find_forward_match(source, start, bracket, matching)
    } else {
        find_backward_match(source, start, bracket, matching)
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

fn is_non_code(source: &Source, byte: usize) -> bool {
    let root = LinkedNode::new(source.root());
    root.leaf_at(byte, Side::After)
        .or_else(|| root.leaf_at(byte, Side::Before))
        .is_some_and(|leaf| {
            node_has_tag(&leaf, |tag| {
                matches!(tag, Tag::Comment | Tag::Raw | Tag::String)
            })
        })
}

fn find_forward_match(
    source: &Source,
    start: usize,
    opening: char,
    closing: char,
) -> Option<usize> {
    let text = source.text();
    let mut depth = 1_usize;
    for (offset, character) in text.get(start + opening.len_utf8()..)?.char_indices() {
        let byte = start + opening.len_utf8() + offset;
        if character != opening && character != closing {
            continue;
        }
        if is_non_code(source, byte) {
            continue;
        }
        if character == opening {
            depth += 1;
        } else {
            depth -= 1;
            if depth == 0 {
                return Some(byte);
            }
        }
    }
    None
}

fn find_backward_match(
    source: &Source,
    start: usize,
    closing: char,
    opening: char,
) -> Option<usize> {
    let text = source.text();
    let mut depth = 1_usize;
    for (byte, character) in text.get(..start)?.char_indices().rev() {
        if character != closing && character != opening {
            continue;
        }
        if is_non_code(source, byte) {
            continue;
        }
        if character == closing {
            depth += 1;
        } else {
            depth -= 1;
            if depth == 0 {
                return Some(byte);
            }
        }
    }
    None
}

fn node_has_tag(node: &LinkedNode<'_>, predicate: impl Fn(Tag) -> bool) -> bool {
    let mut current = Some(node);
    while let Some(node) = current {
        if highlight(node).is_some_and(&predicate) {
            return true;
        }
        current = node.parent();
    }
    false
}

fn effective_style(node: &LinkedNode<'_>, theme: Theme) -> TextStyle {
    let mut tags = Vec::new();
    let mut current = Some(node);
    while let Some(node) = current {
        if let Some(tag) = highlight(node) {
            tags.push(tag);
        }
        current = node.parent();
    }

    tags.into_iter()
        .rev()
        .fold(TextStyle::default(), |style, tag| {
            style.patch(theme.syntax(tag))
        })
}

fn leftmost_leaf_with_trivia(mut node: LinkedNode<'_>) -> LinkedNode<'_> {
    loop {
        let Some(child) = node.children().next() else {
            return node;
        };
        node = child;
    }
}

fn next_leaf_with_trivia<'a>(node: &LinkedNode<'a>) -> Option<LinkedNode<'a>> {
    let mut current = node.clone();
    loop {
        if let Some(next) = current.next_sibling_with_trivia() {
            return Some(leftmost_leaf_with_trivia(next));
        }
        current = current.parent()?.clone();
    }
}

fn styled_ranges(source: &Source, range: Range<usize>, theme: Theme) -> Vec<StyledRange> {
    if range.is_empty() {
        return Vec::new();
    }
    let root = LinkedNode::new(source.root());
    let Some(mut leaf) = root
        .leaf_at(range.start, Side::After)
        .or_else(|| root.leaf_at(range.start, Side::Before))
    else {
        return Vec::new();
    };
    let mut ranges = Vec::new();

    loop {
        let leaf_range = leaf.range();
        if leaf_range.start >= range.end {
            break;
        }
        let start = leaf_range.start.max(range.start);
        let end = leaf_range.end.min(range.end);
        if start < end {
            ranges.push(StyledRange {
                range: start..end,
                style: text_style(effective_style(&leaf, theme)),
            });
        }
        let Some(next) = next_leaf_with_trivia(&leaf) else {
            break;
        };
        leaf = next;
    }

    ranges
}

fn highlighted_range(
    text: &str,
    range: Range<usize>,
    ranges: &[StyledRange],
    first_range: &mut usize,
) -> Line<'static> {
    while ranges
        .get(*first_range)
        .is_some_and(|styled| styled.range.end <= range.start)
    {
        *first_range += 1;
    }
    let mut spans = Vec::new();
    let mut cursor = range.start;
    for styled in ranges.iter().skip(*first_range) {
        if styled.range.start >= range.end {
            break;
        }
        let start = styled.range.start.max(range.start);
        let end = styled.range.end.min(range.end);
        if cursor < start {
            push_span(&mut spans, &text[cursor..start], Style::default());
        }
        if start < end {
            push_span(&mut spans, &text[start..end], styled.style);
            cursor = end;
        }
    }
    if cursor < range.end {
        push_span(&mut spans, &text[cursor..range.end], Style::default());
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
    use std::{convert::Infallible, time::Instant};

    use oxyst_compiler::{Diagnostic, Severity};
    use oxyst_document::Motion;
    use oxyst_theme::{ColorDepth, Theme, ThemeName};
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    use typst_syntax::Source;

    use super::{
        Component, Diagnostics, Editor, VisualPosition, line_row_count, matching_brackets,
    };
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
    fn edits_splice_only_the_affected_line_metrics() -> Result<(), Infallible> {
        let mut editor = Editor::new("abcdefghi\nshort\nthird", theme());
        let mut terminal = Terminal::new(TestBackend::new(12, 6))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

        assert!(editor.set_cursor_line_char(0, 3));
        editor.update(Action::Insert('\n'));

        assert_eq!(editor.line_rows.len(), editor.source.lines().len_lines());
        for line in 0..editor.line_rows.len() {
            assert_eq!(
                editor.line_rows[line],
                line_row_count(&editor.source, line, usize::from(editor.layout_width))
            );
        }
        assert_eq!(
            editor.total_visual_rows,
            editor.line_rows.iter().sum::<usize>()
        );
        Ok(())
    }

    #[test]
    fn edits_before_the_viewport_preserve_the_scroll_anchor() -> Result<(), Infallible> {
        let source = (0..20)
            .map(|line| format!("line-{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut editor = Editor::new(&source, theme());
        let mut terminal = Terminal::new(TestBackend::new(30, 6))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        editor.scroll_lines(10);
        assert_eq!(
            editor.scroll,
            VisualPosition {
                line: 10,
                subrow: 0
            }
        );

        assert!(editor.set_cursor_line_char(0, 6));
        editor.update(Action::Insert('\n'));
        assert_eq!(
            editor.scroll,
            VisualPosition {
                line: 11,
                subrow: 0
            }
        );

        editor.update(Action::Undo);
        assert_eq!(
            editor.scroll,
            VisualPosition {
                line: 10,
                subrow: 0
            }
        );
        Ok(())
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

        assert!(editor.viewport_rows.len() > 1);
        assert!(editor.viewport_rows[1].continuation);
        let continuation_column = editor.viewport_rows[1].start_visual_column;
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
    fn manual_scroll_stays_put_until_the_cursor_moves() -> Result<(), Infallible> {
        let source = (0..12)
            .map(|line| format!("line-{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut editor = Editor::new(&source, theme());
        let mut terminal = Terminal::new(TestBackend::new(24, 6))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

        editor.scroll_lines(3);
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert_eq!(editor.scroll, VisualPosition { line: 3, subrow: 0 });
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!rendered.contains("line-0"));
        assert!(rendered.contains("line-3"));

        editor.update(Action::Move(Motion::DocumentEnd));
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert_eq!(
            editor.scroll,
            editor.max_scroll_position(editor.text_area.height)
        );
        Ok(())
    }

    #[test]
    fn viewport_cache_stays_bounded_for_large_and_long_sources() -> Result<(), Infallible> {
        let source = (0..2_000)
            .map(|line| format!("#let value{line} = {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut editor = Editor::new(&source, theme());
        let mut terminal = Terminal::new(TestBackend::new(40, 8))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

        assert_eq!(editor.line_rows.len(), 2_000);
        assert!(editor.viewport_rows.len() <= usize::from(editor.text_area.height));
        assert!(editor.set_cursor_line_char(1_999, 0));
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert_eq!(
            editor.viewport_rows.last().map(|row| row.logical_line),
            Some(1_999)
        );
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.fg == Color::Blue)
        );

        let long_source = "a".repeat(100_000);
        let mut long_editor = Editor::new(&long_source, theme());
        terminal.draw(|frame| long_editor.draw(frame, frame.area(), true))?;
        let materialized_bytes = long_editor
            .viewport_rows
            .iter()
            .flat_map(|row| &row.content.spans)
            .map(|span| span.content.len())
            .sum::<usize>();
        assert!(materialized_bytes < long_source.len());
        assert!(long_editor.viewport_rows.len() <= usize::from(long_editor.text_area.height));
        Ok(())
    }

    #[test]
    fn bracket_matching_ignores_strings() -> Result<(), &'static str> {
        let text = "#let value = (\"ignored )\" + [1])";
        let source = Source::detached(text);
        let opening = text.find('(').ok_or("opening bracket is missing")?;
        let closing = text.rfind(')').ok_or("closing bracket is missing")?;
        let matched = matching_brackets(&source, opening).ok_or("outer brackets did not match")?;
        assert_eq!(matched[1].start, closing);

        let string_bracket = text.find(")\"").ok_or("string bracket is missing")?;
        assert!(matching_brackets(&source, string_bracket).is_none());
        Ok(())
    }

    #[test]
    #[ignore = "manual release-mode performance probe"]
    fn massive_document_edit_and_draw_is_viewport_bound() -> Result<(), Infallible> {
        let source = (0..100_000)
            .map(|line| format!("#let value{line} = ({line} + 1)"))
            .collect::<Vec<_>>()
            .join("\n");
        let started = Instant::now();
        let mut editor = Editor::new(&source, theme());
        let initialized = started.elapsed();
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        let laid_out = started.elapsed();
        assert!(editor.set_cursor_line_char(99_999, 0));
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

        let mut edit_samples = Vec::new();
        let mut draw_samples = Vec::new();
        for _ in 0..20 {
            let edit_started = Instant::now();
            editor.update(Action::Insert('x'));
            edit_samples.push(edit_started.elapsed());
            let draw_started = Instant::now();
            terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
            draw_samples.push(draw_started.elapsed());
        }
        edit_samples.sort_unstable();
        draw_samples.sort_unstable();
        let edit_p95 = edit_samples[edit_samples.len() * 95 / 100];
        let draw_p95 = draw_samples[draw_samples.len() * 95 / 100];

        eprintln!(
            "100k token-dense lines: init={initialized:?}, initial_layout={:?}, edit_p95={edit_p95:?}, draw_p95={draw_p95:?}",
            laid_out.saturating_sub(initialized)
        );
        assert_eq!(editor.line_rows.len(), 100_000);
        assert!(editor.viewport_rows.len() <= usize::from(editor.text_area.height));
        Ok(())
    }
}
