#![forbid(unsafe_code)]

use std::{collections::VecDeque, ops::Range};

use ropey::{Rope, RopeSlice};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

const HISTORY_LIMIT: usize = 256;
const HISTORY_BYTE_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorPosition {
    pub line: usize,
    pub column: usize,
    pub visual_column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    range: Range<usize>,
    replacement: String,
}

impl TextEdit {
    #[must_use]
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    #[must_use]
    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

#[derive(Debug)]
struct Edit {
    start: usize,
    removed: String,
    inserted: String,
    cursor_before: usize,
    cursor_after: usize,
    state_before: u64,
    state_after: u64,
}

impl Edit {
    fn retained_bytes(&self) -> usize {
        self.removed.len().saturating_add(self.inserted.len())
    }
}

#[derive(Debug)]
pub struct Document {
    text: Rope,
    cursor: usize,
    cursor_position: CursorPosition,
    selection_anchor: Option<usize>,
    preferred_visual_column: Option<usize>,
    undo: VecDeque<Edit>,
    redo: Vec<Edit>,
    state: u64,
    saved_state: u64,
    next_state: u64,
    last_edit: Option<TextEdit>,
    selection_grapheme_count: usize,
}

impl Document {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            text: Rope::from_str(text),
            cursor: 0,
            cursor_position: CursorPosition {
                line: 0,
                column: 0,
                visual_column: 0,
            },
            selection_anchor: None,
            preferred_visual_column: None,
            undo: VecDeque::new(),
            redo: Vec::new(),
            state: 0,
            saved_state: 0,
            next_state: 1,
            last_edit: None,
            selection_grapheme_count: 0,
        }
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.text.to_string()
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.text.len_lines()
    }

    #[must_use]
    pub fn line(&self, line: usize) -> Option<String> {
        (line < self.line_count()).then(|| self.line_without_ending(line))
    }

    #[must_use]
    pub fn cursor_char_index(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn cursor_byte_index(&self) -> usize {
        self.text.char_to_byte(self.cursor)
    }

    #[must_use]
    pub fn cursor_position(&self) -> CursorPosition {
        self.cursor_position
    }

    #[must_use]
    pub fn selection_char_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor?;
        (anchor != self.cursor).then(|| anchor.min(self.cursor)..anchor.max(self.cursor))
    }

    #[must_use]
    pub fn selection_byte_range(&self) -> Option<Range<usize>> {
        self.selection_char_range()
            .map(|range| self.text.char_to_byte(range.start)..self.text.char_to_byte(range.end))
    }

    #[must_use]
    pub fn selected_text(&self) -> Option<String> {
        self.selection_char_range()
            .map(|range| self.text.slice(range).to_string())
    }

    #[must_use]
    pub fn selection_graphemes(&self) -> usize {
        self.selection_grapheme_count
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.state != self.saved_state
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.state
    }

    #[must_use]
    pub fn last_edit(&self) -> Option<&TextEdit> {
        self.last_edit.as_ref()
    }

    pub fn set_cursor_byte_index(&mut self, byte: usize) -> bool {
        if byte > self.text.len_bytes() {
            return false;
        }
        let cursor = self.text.byte_to_char(byte);
        if self.text.char_to_byte(cursor) != byte {
            return false;
        }

        self.cursor = cursor;
        self.recalculate_cursor_position();
        self.clear_selection();
        self.preferred_visual_column = None;
        true
    }

    pub fn set_cursor_line_char(&mut self, line: usize, column: usize) -> bool {
        if line >= self.line_count() {
            return false;
        }

        let line_start = self.text.line_to_char(line);
        let line_length = self.line_without_ending(line).chars().count();
        self.cursor = line_start + column.min(line_length);
        self.recalculate_cursor_position();
        self.clear_selection();
        self.preferred_visual_column = None;
        true
    }

    pub fn set_cursor_visual_position(
        &mut self,
        line: usize,
        visual_column: usize,
        selecting: bool,
    ) -> bool {
        if line >= self.line_count() {
            return false;
        }

        let anchor = selecting.then_some(self.selection_anchor.unwrap_or(self.cursor));
        let old_cursor = self.cursor;
        let text = self.line_without_ending(line);
        self.cursor =
            self.text.line_to_char(line) + char_offset_at_visual_column(&text, visual_column);
        self.recalculate_cursor_position();
        self.selection_anchor = anchor.filter(|anchor| *anchor != self.cursor);
        self.update_selection_graphemes(anchor, old_cursor);
        self.preferred_visual_column = None;
        true
    }

    pub fn select_all(&mut self) {
        if self.text.len_chars() == 0 {
            self.clear_selection();
            return;
        }
        self.selection_anchor = Some(0);
        self.cursor = self.text.len_chars();
        self.recalculate_cursor_position();
        self.recount_selection_graphemes();
        self.preferred_visual_column = None;
    }

    pub fn select_byte_range(&mut self, range: Range<usize>) -> bool {
        if range.start > range.end || range.end > self.text.len_bytes() {
            return false;
        }
        let start = self.text.byte_to_char(range.start);
        let end = self.text.byte_to_char(range.end);
        if self.text.char_to_byte(start) != range.start || self.text.char_to_byte(end) != range.end
        {
            return false;
        }

        self.selection_anchor = (start != end).then_some(start);
        self.cursor = end;
        self.recalculate_cursor_position();
        self.recount_selection_graphemes();
        self.preferred_visual_column = None;
        true
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
        self.selection_grapheme_count = 0;
    }

    pub fn mark_saved(&mut self) {
        self.saved_state = self.state;
    }

    pub fn insert_char(&mut self, character: char) {
        let mut encoded = [0; 4];
        self.insert_text(character.encode_utf8(&mut encoded));
    }

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let range = self
            .selection_char_range()
            .unwrap_or(self.cursor..self.cursor);
        self.record_edit(range.start, range.end, text);
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }

        let line = self.text.char_to_line(self.cursor);
        let line_start = self.text.line_to_char(line);
        let start = if self.cursor == line_start {
            self.line_content_end(line - 1)
        } else {
            self.previous_grapheme_boundary(line_start, self.cursor)
        };

        self.record_edit(start, self.cursor, "");
    }

    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == self.text.len_chars() {
            return;
        }

        let line = self.text.char_to_line(self.cursor);
        let content_end = self.line_content_end(line);
        let end = if self.cursor == content_end && line + 1 < self.line_count() {
            self.text.line_to_char(line + 1)
        } else {
            self.next_grapheme_boundary(line, self.cursor)
        };

        self.record_edit(self.cursor, end, "");
    }

    pub fn move_cursor(&mut self, motion: Motion) {
        self.move_cursor_selecting(motion, false);
    }

    pub fn move_cursor_selecting(&mut self, motion: Motion, selecting: bool) {
        if !selecting && let Some(range) = self.selection_char_range() {
            self.cursor = match motion {
                Motion::Left | Motion::Up | Motion::WordLeft => range.start,
                Motion::Right | Motion::Down | Motion::WordRight => range.end,
                Motion::LineStart => self.text.line_to_char(self.text.char_to_line(self.cursor)),
                Motion::LineEnd => self.line_content_end(self.text.char_to_line(self.cursor)),
                Motion::DocumentStart => 0,
                Motion::DocumentEnd => self.text.len_chars(),
            };
            self.recalculate_cursor_position();
            self.clear_selection();
            self.preferred_visual_column = None;
            return;
        }

        let old_cursor = self.cursor;
        let old_position = self.cursor_position;
        let anchor = selecting.then_some(self.selection_anchor.unwrap_or(old_cursor));
        let vertical = matches!(motion, Motion::Up | Motion::Down);
        match motion {
            Motion::Left => self.cursor = self.cursor_left(),
            Motion::Right => self.cursor = self.cursor_right(),
            Motion::Up => self.move_vertically(-1),
            Motion::Down => self.move_vertically(1),
            Motion::WordLeft => self.cursor = self.word_left(),
            Motion::WordRight => self.cursor = self.word_right(),
            Motion::LineStart => {
                let line = self.text.char_to_line(self.cursor);
                self.cursor = self.text.line_to_char(line);
            }
            Motion::LineEnd => {
                let line = self.text.char_to_line(self.cursor);
                self.cursor = self.line_content_end(line);
            }
            Motion::DocumentStart => self.cursor = 0,
            Motion::DocumentEnd => self.cursor = self.text.len_chars(),
        }

        self.selection_anchor = anchor.filter(|anchor| *anchor != self.cursor);
        self.update_cursor_position_after_motion(old_cursor, old_position, motion);
        self.update_selection_graphemes(anchor, old_cursor);
        if !vertical {
            self.preferred_visual_column = None;
        }
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_char_range() else {
            return false;
        };
        self.record_edit(range.start, range.end, "");
        true
    }

    pub fn undo(&mut self) {
        let Some(edit) = self.undo.pop_back() else {
            return;
        };

        let inserted_end = edit.start + edit.inserted.chars().count();
        let range = self.text.char_to_byte(edit.start)..self.text.char_to_byte(inserted_end);
        self.replace_text(edit.start, inserted_end, &edit.removed);
        self.cursor = edit.cursor_before;
        self.recalculate_cursor_position();
        self.clear_selection();
        self.state = edit.state_before;
        self.preferred_visual_column = None;
        self.last_edit = Some(TextEdit {
            range,
            replacement: edit.removed.clone(),
        });
        self.redo.push(edit);
    }

    pub fn redo(&mut self) {
        let Some(edit) = self.redo.pop() else {
            return;
        };

        let removed_end = edit.start + edit.removed.chars().count();
        let range = self.text.char_to_byte(edit.start)..self.text.char_to_byte(removed_end);
        self.replace_text(edit.start, removed_end, &edit.inserted);
        self.cursor = edit.cursor_after;
        self.recalculate_cursor_position();
        self.clear_selection();
        self.state = edit.state_after;
        self.preferred_visual_column = None;
        self.last_edit = Some(TextEdit {
            range,
            replacement: edit.inserted.clone(),
        });
        self.push_undo(edit);
    }

    fn record_edit(&mut self, start: usize, end: usize, inserted: &str) {
        let range = self.text.char_to_byte(start)..self.text.char_to_byte(end);
        let removed = self.text.slice(start..end).to_string();
        let cursor_before = self.cursor;
        let position_before = self.cursor_position;
        let cursor_after = start + inserted.chars().count();
        let state_before = self.state;
        let state_after = self.next_state;
        self.next_state = self.next_state.wrapping_add(1);

        self.replace_text(start, end, inserted);
        self.cursor = cursor_after;
        self.update_cursor_position_after_edit(
            start,
            end,
            inserted,
            &removed,
            cursor_before,
            position_before,
        );
        self.clear_selection();
        self.state = state_after;
        self.preferred_visual_column = None;
        self.redo.clear();
        self.last_edit = Some(TextEdit {
            range,
            replacement: inserted.to_owned(),
        });
        self.push_undo(Edit {
            start,
            removed,
            inserted: inserted.to_owned(),
            cursor_before,
            cursor_after,
            state_before,
            state_after,
        });
    }

    fn push_undo(&mut self, edit: Edit) {
        let edit_bytes = edit.retained_bytes();
        let mut retained_bytes = self.undo.iter().map(Edit::retained_bytes).sum::<usize>();
        while !self.undo.is_empty()
            && (self.undo.len() >= HISTORY_LIMIT
                || retained_bytes.saturating_add(edit_bytes) > HISTORY_BYTE_LIMIT)
        {
            if let Some(discarded) = self.undo.pop_front() {
                retained_bytes = retained_bytes.saturating_sub(discarded.retained_bytes());
            }
        }
        self.undo.push_back(edit);
    }

    fn recount_selection_graphemes(&mut self) {
        self.selection_grapheme_count = self
            .selection_char_range()
            .map_or(0, |range| count_graphemes(self.text.slice(range)));
    }

    fn update_selection_graphemes(&mut self, anchor: Option<usize>, old_cursor: usize) {
        let Some(anchor) = anchor else {
            self.selection_grapheme_count = 0;
            return;
        };
        let new_cursor = self.cursor;
        if new_cursor == anchor {
            self.selection_grapheme_count = 0;
            return;
        }
        if old_cursor == anchor || (old_cursor < anchor) != (new_cursor < anchor) {
            let range = anchor.min(new_cursor)..anchor.max(new_cursor);
            self.selection_grapheme_count = count_graphemes(self.text.slice(range));
            return;
        }

        let range = old_cursor.min(new_cursor)..old_cursor.max(new_cursor);
        let delta = count_graphemes(self.text.slice(range));
        if old_cursor.abs_diff(anchor) < new_cursor.abs_diff(anchor) {
            self.selection_grapheme_count = self.selection_grapheme_count.saturating_add(delta);
        } else {
            self.selection_grapheme_count = self.selection_grapheme_count.saturating_sub(delta);
        }
    }

    fn recalculate_cursor_position(&mut self) {
        let line = self.text.char_to_line(self.cursor);
        let line_start = self.text.line_to_char(line);
        let before_cursor = self.text.slice(line_start..self.cursor).to_string();
        self.cursor_position = CursorPosition {
            line,
            column: before_cursor.graphemes(true).count(),
            visual_column: UnicodeWidthStr::width(before_cursor.as_str()),
        };
    }

    fn update_cursor_position_after_motion(
        &mut self,
        old_cursor: usize,
        old_position: CursorPosition,
        motion: Motion,
    ) {
        if self.cursor == old_cursor {
            return;
        }
        if matches!(motion, Motion::Left | Motion::Right)
            && self.text.char_to_line(self.cursor) == old_position.line
        {
            let range = old_cursor.min(self.cursor)..old_cursor.max(self.cursor);
            let moved = self.text.slice(range).to_string();
            let columns = moved.graphemes(true).count();
            let visual = UnicodeWidthStr::width(moved.as_str());
            self.cursor_position = if self.cursor > old_cursor {
                CursorPosition {
                    line: old_position.line,
                    column: old_position.column.saturating_add(columns),
                    visual_column: old_position.visual_column.saturating_add(visual),
                }
            } else {
                CursorPosition {
                    line: old_position.line,
                    column: old_position.column.saturating_sub(columns),
                    visual_column: old_position.visual_column.saturating_sub(visual),
                }
            };
        } else {
            self.recalculate_cursor_position();
        }
    }

    fn update_cursor_position_after_edit(
        &mut self,
        start: usize,
        end: usize,
        inserted: &str,
        removed: &str,
        cursor_before: usize,
        position_before: CursorPosition,
    ) {
        let single_line_ascii = |text: &str| text.is_ascii() && !text.contains(['\r', '\n']);
        let line_start = self.text.line_to_char(self.text.char_to_line(start));
        let left_context_is_ascii = start == line_start || self.text.char(start - 1).is_ascii();
        if start == end
            && start == cursor_before
            && single_line_ascii(inserted)
            && left_context_is_ascii
        {
            self.cursor_position = CursorPosition {
                line: position_before.line,
                column: position_before
                    .column
                    .saturating_add(inserted.chars().count()),
                visual_column: position_before
                    .visual_column
                    .saturating_add(UnicodeWidthStr::width(inserted)),
            };
        } else if inserted.is_empty()
            && end == cursor_before
            && start < end
            && single_line_ascii(removed)
            && left_context_is_ascii
        {
            self.cursor_position = CursorPosition {
                line: position_before.line,
                column: position_before
                    .column
                    .saturating_sub(removed.chars().count()),
                visual_column: position_before
                    .visual_column
                    .saturating_sub(UnicodeWidthStr::width(removed)),
            };
        } else if inserted.is_empty() && start == cursor_before {
            self.cursor_position = position_before;
        } else {
            self.recalculate_cursor_position();
        }
    }

    fn replace_text(&mut self, start: usize, end: usize, inserted: &str) {
        self.text.remove(start..end);
        self.text.insert(start, inserted);
    }

    fn move_vertically(&mut self, direction: isize) {
        let position = self.cursor_position();
        let preferred = self
            .preferred_visual_column
            .unwrap_or(position.visual_column);
        let target_line = position
            .line
            .saturating_add_signed(direction)
            .min(self.line_count() - 1);
        let target_text = self.line_without_ending(target_line);
        let char_offset = char_offset_at_visual_column(&target_text, preferred);

        self.cursor = self.text.line_to_char(target_line) + char_offset;
        self.preferred_visual_column = Some(preferred);
    }

    fn cursor_left(&self) -> usize {
        let line = self.text.char_to_line(self.cursor);
        let line_start = self.text.line_to_char(line);
        if self.cursor == line_start {
            return line
                .checked_sub(1)
                .map_or(0, |previous_line| self.line_content_end(previous_line));
        }

        self.previous_grapheme_boundary(line_start, self.cursor)
    }

    fn cursor_right(&self) -> usize {
        let line = self.text.char_to_line(self.cursor);
        let content_end = self.line_content_end(line);
        if self.cursor == content_end {
            return if line + 1 < self.line_count() {
                self.text.line_to_char(line + 1)
            } else {
                content_end
            };
        }

        self.next_grapheme_boundary(line, self.cursor)
    }

    fn word_left(&self) -> usize {
        let mut line = self.text.char_to_line(self.cursor);
        let mut end = self.cursor;
        loop {
            let start = self.text.line_to_char(line);
            if start < end {
                let prefix = self.text.slice(start..end).to_string();
                if let Some((byte, _)) = prefix.unicode_word_indices().next_back() {
                    return start + prefix[..byte].chars().count();
                }
            }
            let Some(previous) = line.checked_sub(1) else {
                return 0;
            };
            line = previous;
            end = self.line_content_end(line);
        }
    }

    fn word_right(&self) -> usize {
        let mut line = self.text.char_to_line(self.cursor);
        while line < self.line_count() {
            let start = self.text.line_to_char(line);
            let end = self.line_content_end(line);
            let content = self.text.slice(start..end).to_string();
            if let Some(position) = content.unicode_word_indices().find_map(|(byte, _)| {
                let position = start + content[..byte].chars().count();
                (position > self.cursor).then_some(position)
            }) {
                return position;
            }
            line += 1;
        }
        self.text.len_chars()
    }

    fn previous_grapheme_boundary(&self, line_start: usize, cursor: usize) -> usize {
        let line = self.text.char_to_line(cursor);
        let content = self.text.slice(self.line_content_char_range(line));
        let cursor_byte = content.char_to_byte(cursor - line_start);
        let boundary = rope_grapheme_boundary(content, cursor_byte, false).unwrap_or(0);
        line_start + content.byte_to_char(boundary)
    }

    fn next_grapheme_boundary(&self, line: usize, cursor: usize) -> usize {
        let range = self.line_content_char_range(line);
        let content = self.text.slice(range.clone());
        let cursor_byte = content.char_to_byte(cursor - range.start);
        let boundary =
            rope_grapheme_boundary(content, cursor_byte, true).unwrap_or(content.len_bytes());
        range.start + content.byte_to_char(boundary)
    }

    fn line_content_end(&self, line: usize) -> usize {
        self.line_content_char_range(line).end
    }

    fn line_content_char_range(&self, line: usize) -> Range<usize> {
        let start = self.text.line_to_char(line);
        let mut end = if line + 1 < self.line_count() {
            self.text.line_to_char(line + 1)
        } else {
            self.text.len_chars()
        };
        while end > start && is_line_ending(self.text.char(end - 1)) {
            end -= 1;
        }
        start..end
    }

    fn line_without_ending(&self, line: usize) -> String {
        self.text
            .slice(self.line_content_char_range(line))
            .to_string()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new("")
    }
}

fn is_line_ending(character: char) -> bool {
    matches!(
        character,
        '\r' | '\n' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

fn rope_grapheme_boundary(text: RopeSlice<'_>, byte: usize, forward: bool) -> Option<usize> {
    let len = text.len_bytes();
    if byte > len {
        return None;
    }
    if (forward && byte == len) || (!forward && byte == 0) {
        return Some(byte);
    }

    let mut cursor = GraphemeCursor::new(byte, len, true);
    let mut chunk_byte = if forward { byte } else { byte - 1 };
    loop {
        let (chunk, chunk_start, _, _) = text.chunk_at_byte(chunk_byte);
        let boundary = if forward {
            cursor.next_boundary(chunk, chunk_start)
        } else {
            cursor.prev_boundary(chunk, chunk_start)
        };
        match boundary {
            Ok(boundary) => return boundary,
            Err(GraphemeIncomplete::NextChunk) => {
                chunk_byte = chunk_start.checked_add(chunk.len())?;
                if chunk_byte >= len {
                    return Some(len);
                }
            }
            Err(GraphemeIncomplete::PrevChunk) => {
                chunk_byte = chunk_start.checked_sub(1)?;
            }
            Err(GraphemeIncomplete::PreContext(offset)) => {
                let context_byte = offset.checked_sub(1)?;
                let (context, context_start, _, _) = text.chunk_at_byte(context_byte);
                let context_end = offset.checked_sub(context_start)?;
                cursor.provide_context(context.get(..context_end)?, context_start);
            }
            Err(GraphemeIncomplete::InvalidOffset) => return None,
        }
    }
}

fn count_graphemes(text: RopeSlice<'_>) -> usize {
    let len = text.len_bytes();
    if len == 0 {
        return 0;
    }
    if let Some(text) = text.as_str() {
        return text.graphemes(true).count();
    }
    let mut chunk_start = 0_usize;
    let chunks = text
        .chunks()
        .map(|chunk| {
            let start = chunk_start;
            chunk_start += chunk.len();
            (start, chunk)
        })
        .collect::<Vec<_>>();
    let mut cursor = GraphemeCursor::new(0, len, true);
    let mut chunk_index = 0_usize;
    let mut count = 0_usize;

    loop {
        let Some(&(start, chunk)) = chunks.get(chunk_index) else {
            return text.to_string().graphemes(true).count();
        };
        match cursor.next_boundary(chunk, start) {
            Ok(Some(_)) => count += 1,
            Ok(None) => return count,
            Err(GraphemeIncomplete::NextChunk) => chunk_index += 1,
            Err(GraphemeIncomplete::PreContext(offset)) => {
                let Some(context_index) = chunks
                    .partition_point(|(context_start, _)| *context_start < offset)
                    .checked_sub(1)
                else {
                    return text.to_string().graphemes(true).count();
                };
                let Some(&(context_start, context)) = chunks.get(context_index) else {
                    return text.to_string().graphemes(true).count();
                };
                let Some(context_end) = offset.checked_sub(context_start) else {
                    return text.to_string().graphemes(true).count();
                };
                let Some(context) = context.get(..context_end) else {
                    return text.to_string().graphemes(true).count();
                };
                cursor.provide_context(context, context_start);
            }
            Err(GraphemeIncomplete::PrevChunk | GraphemeIncomplete::InvalidOffset) => {
                return text.to_string().graphemes(true).count();
            }
        }
    }
}

fn char_offset_at_visual_column(text: &str, target: usize) -> usize {
    let mut width = 0;
    let mut chars = 0;

    for grapheme in text.graphemes(true) {
        let next_width = width + UnicodeWidthStr::width(grapheme);
        if next_width > target {
            break;
        }
        width = next_width;
        chars += grapheme.chars().count();
    }

    chars
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ropey::RopeSlice;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    use super::{
        CursorPosition, Document, Edit, HISTORY_BYTE_LIMIT, Motion, TextEdit, count_graphemes,
        rope_grapheme_boundary,
    };

    #[test]
    fn edits_undo_and_redo() {
        let mut document = Document::new("Hello");
        document.move_cursor(Motion::DocumentEnd);
        document.insert_text(", world");
        document.backspace();

        assert_eq!(document.text(), "Hello, worl");

        document.undo();
        assert_eq!(document.text(), "Hello, world");

        document.undo();
        assert_eq!(document.text(), "Hello");

        document.redo();
        document.redo();
        assert_eq!(document.text(), "Hello, worl");
    }

    #[test]
    fn horizontal_motion_uses_grapheme_boundaries() {
        let mut document = Document::new("a\u{301}界\nb");

        document.move_cursor(Motion::Right);
        assert_eq!(document.cursor_char_index(), 2);
        assert_eq!(
            document.cursor_position(),
            CursorPosition {
                line: 0,
                column: 1,
                visual_column: 1,
            }
        );

        document.move_cursor(Motion::Right);
        assert_eq!(document.cursor_char_index(), 3);
        assert_eq!(document.cursor_position().visual_column, 3);

        document.move_cursor(Motion::Down);
        assert_eq!(document.cursor_position().line, 1);
        assert_eq!(document.cursor_position().visual_column, 1);

        document.move_cursor(Motion::Up);
        assert_eq!(document.cursor_position().visual_column, 3);
    }

    #[test]
    fn deletion_treats_crlf_as_one_line_break() {
        let mut document = Document::new("one\r\ntwo");
        document.move_cursor(Motion::LineEnd);
        document.move_cursor(Motion::Right);
        document.backspace();

        assert_eq!(document.text(), "onetwo");
    }

    #[test]
    fn dirty_state_tracks_saved_content_through_history() {
        let mut document = Document::new("text");
        document.mark_saved();
        document.insert_char('!');
        assert!(document.is_dirty());

        document.undo();
        assert!(!document.is_dirty());

        document.redo();
        assert!(document.is_dirty());

        document.mark_saved();
        assert!(!document.is_dirty());
    }

    #[test]
    fn undo_history_is_bounded() {
        let mut document = Document::default();
        for _ in 0..300 {
            document.insert_char('x');
        }
        for _ in 0..300 {
            document.undo();
        }

        assert_eq!(document.text().chars().count(), 44);
    }

    #[test]
    fn undo_history_is_bounded_by_retained_text() {
        let mut document = Document::default();
        for state in 0..10 {
            document.push_undo(Edit {
                start: 0,
                removed: String::new(),
                inserted: "x".repeat(2 * 1024 * 1024),
                cursor_before: 0,
                cursor_after: 0,
                state_before: state,
                state_after: state + 1,
            });
        }

        let retained = document
            .undo
            .iter()
            .map(Edit::retained_bytes)
            .sum::<usize>();
        assert!(retained <= HISTORY_BYTE_LIMIT);
    }

    #[test]
    fn edit_deltas_use_utf8_byte_ranges() {
        let mut document = Document::new("aé");
        document.move_cursor(Motion::DocumentEnd);
        document.insert_char('界');

        assert_eq!(document.last_edit().map(TextEdit::range), Some(3..3));
        assert_eq!(document.last_edit().map(TextEdit::replacement), Some("界"));

        document.backspace();
        assert_eq!(document.last_edit().map(TextEdit::range), Some(3..6));
        assert_eq!(document.last_edit().map(TextEdit::replacement), Some(""));

        document.undo();
        assert_eq!(document.last_edit().map(TextEdit::range), Some(3..3));
        assert_eq!(document.last_edit().map(TextEdit::replacement), Some("界"));
    }

    #[test]
    fn cursor_placement_validates_utf8_boundaries() {
        let mut document = Document::new("aé\n界");

        assert!(document.set_cursor_byte_index(3));
        assert_eq!(document.cursor_position().line, 0);
        assert_eq!(document.cursor_position().column, 2);
        assert!(!document.set_cursor_byte_index(2));
        assert_eq!(document.cursor_byte_index(), 3);

        assert!(document.set_cursor_line_char(1, 99));
        assert_eq!(document.cursor_position().line, 1);
        assert_eq!(document.cursor_position().column, 1);
        assert!(!document.set_cursor_line_char(2, 0));
    }

    #[test]
    fn selection_is_grapheme_correct_and_replaced_as_one_edit() {
        let mut document = Document::new("a\u{301}界z");
        document.move_cursor_selecting(Motion::Right, true);
        document.move_cursor_selecting(Motion::Right, true);

        assert_eq!(document.selected_text().as_deref(), Some("a\u{301}界"));
        assert_eq!(document.selection_graphemes(), 2);

        document.insert_text("X");
        assert_eq!(document.text(), "Xz");
        document.undo();
        assert_eq!(document.text(), "a\u{301}界z");
    }

    #[test]
    fn selection_count_updates_when_growing_shrinking_and_crossing_the_anchor() {
        let mut document = Document::new("a\u{301}界z");
        document.move_cursor(Motion::Right);
        document.move_cursor_selecting(Motion::Right, true);
        assert_eq!(document.selection_graphemes(), 1);

        document.move_cursor_selecting(Motion::Left, true);
        assert_eq!(document.selection_graphemes(), 0);
        document.move_cursor_selecting(Motion::Left, true);
        assert_eq!(document.selection_graphemes(), 1);
        document.move_cursor_selecting(Motion::Right, true);
        assert_eq!(document.selection_graphemes(), 0);
    }

    #[test]
    fn chunked_grapheme_count_matches_contiguous_unicode_segmentation() {
        let source = (0..4_096)
            .map(|index| {
                if index % 3 == 0 {
                    "a\u{301}"
                } else if index % 3 == 1 {
                    "👩‍💻"
                } else {
                    "界"
                }
            })
            .collect::<String>();
        let document = Document::new(&source);

        assert_eq!(
            count_graphemes(document.text.slice(..)),
            source.graphemes(true).count()
        );
    }

    #[test]
    fn cached_cursor_position_tracks_common_long_line_edits_and_motion() {
        let source = "x".repeat(100_000);
        let mut document = Document::new(&source);
        document.move_cursor(Motion::DocumentEnd);
        document.insert_char('y');
        assert_eq!(document.cursor_position().column, 100_001);

        document.backspace();
        document.move_cursor(Motion::Left);
        assert_eq!(document.cursor_position().column, 99_999);
        document.move_cursor(Motion::Right);
        assert_eq!(document.cursor_position().column, 100_000);

        let mut joining_context = Document::new("\u{600}");
        joining_context.move_cursor(Motion::DocumentEnd);
        joining_context.insert_char('a');
        assert_eq!(joining_context.cursor_position().column, 1);
        joining_context.backspace();
        assert_eq!(joining_context.cursor_position().column, 0);
    }

    #[test]
    fn horizontal_motion_crosses_rope_chunks_at_grapheme_boundaries() {
        let graphemes = ["a\u{301}", "👩‍💻", "界"];
        let source = (0..4_096)
            .map(|index| graphemes[index % graphemes.len()])
            .collect::<String>();
        let mut document = Document::new(&source);
        document.move_cursor(Motion::DocumentEnd);

        for _ in 0..4_096 {
            document.move_cursor(Motion::Left);
        }
        assert_eq!(document.cursor_char_index(), 0);
        for _ in 0..4_096 {
            document.move_cursor(Motion::Right);
        }
        assert_eq!(document.cursor_char_index(), source.chars().count());
    }

    #[test]
    #[ignore = "manual release-mode performance probe"]
    fn long_line_cursor_and_selection_growth_stay_incremental() {
        let source = "x".repeat(250_000);
        let mut document = Document::new(&source);
        let started = Instant::now();
        for _ in 0..250_000 {
            document.move_cursor_selecting(Motion::Right, true);
        }
        let selection = started.elapsed();
        document.move_cursor(Motion::Right);
        let started = Instant::now();
        for _ in 0..10_000 {
            document.move_cursor(Motion::Left);
            document.move_cursor(Motion::Right);
        }
        let cursor = started.elapsed();

        eprintln!(
            "250k-grapheme line: selection_growth={selection:?}, 20k cursor_moves={cursor:?}"
        );
        assert_eq!(document.selection_graphemes(), 0);
        assert_eq!(document.cursor_char_index(), source.len());
    }

    #[test]
    #[ignore = "manual release-mode performance probe"]
    fn massive_destination_line_reports_vertical_motion_latency() {
        let long_line = "x".repeat(5 * 1024 * 1024);
        let mut document = Document::new(&format!("{long_line}\nshort"));
        document.move_cursor(Motion::DocumentEnd);

        let started = Instant::now();
        document.move_cursor(Motion::Up);
        let materialized = started.elapsed();
        assert_eq!(document.cursor_position().visual_column, 5);

        let line = document.text.line(0);
        let started = Instant::now();
        let offset = rope_char_offset_at_visual_column(line, 5);
        let traversed = started.elapsed();
        assert_eq!(offset, 5);

        eprintln!(
            "5 MiB destination line at column 5: materialized={materialized:?}, chunk_traversal={traversed:?}"
        );
    }

    fn rope_char_offset_at_visual_column(text: RopeSlice<'_>, target: usize) -> usize {
        let mut byte = 0_usize;
        let mut width = 0_usize;
        let mut chars = 0_usize;
        while byte < text.len_bytes() {
            let Some(next) = rope_grapheme_boundary(text, byte, true) else {
                break;
            };
            let grapheme = text.byte_slice(byte..next).to_string();
            let next_width = width.saturating_add(UnicodeWidthStr::width(grapheme.as_str()));
            if next_width > target {
                break;
            }
            width = next_width;
            chars = chars.saturating_add(text.byte_slice(byte..next).len_chars());
            byte = next;
        }
        chars
    }

    #[test]
    fn vertical_selection_keeps_its_anchor_and_preferred_column() {
        let mut document = Document::new("abcd\nx\nabcd");
        assert!(document.set_cursor_line_char(0, 3));

        document.move_cursor_selecting(Motion::Down, true);
        document.move_cursor_selecting(Motion::Down, true);

        assert_eq!(document.cursor_position().line, 2);
        assert_eq!(document.cursor_position().visual_column, 3);
        assert_eq!(document.selected_text().as_deref(), Some("d\nx\nabc"));
    }

    #[test]
    fn word_motion_scans_lines_without_changing_boundaries() {
        let mut document = Document::new("alpha\n\nβeta gamma");

        document.move_cursor(Motion::WordRight);
        assert_eq!(document.cursor_position().line, 2);
        assert_eq!(document.cursor_position().column, 0);

        document.move_cursor(Motion::DocumentEnd);
        document.move_cursor(Motion::WordLeft);
        assert_eq!(document.cursor_position().column, 5);
        document.move_cursor(Motion::WordLeft);
        assert_eq!(document.cursor_position().column, 0);
        document.move_cursor(Motion::WordLeft);
        assert_eq!(document.cursor_position().line, 0);
        assert_eq!(document.cursor_position().column, 0);
    }

    #[test]
    fn select_all_and_delete_are_undoable() {
        let mut document = Document::new("one\ntwo");
        document.select_all();
        assert_eq!(document.selected_text().as_deref(), Some("one\ntwo"));
        assert!(document.delete_selection());
        assert_eq!(document.text(), "");

        document.undo();
        assert_eq!(document.text(), "one\ntwo");
    }
}
