#![forbid(unsafe_code)]

use std::collections::VecDeque;

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const HISTORY_LIMIT: usize = 256;

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

#[derive(Debug)]
pub struct Document {
    text: Rope,
    cursor: usize,
    preferred_visual_column: Option<usize>,
    undo: VecDeque<Edit>,
    redo: Vec<Edit>,
    state: u64,
    saved_state: u64,
    next_state: u64,
}

impl Document {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            text: Rope::from_str(text),
            cursor: 0,
            preferred_visual_column: None,
            undo: VecDeque::new(),
            redo: Vec::new(),
            state: 0,
            saved_state: 0,
            next_state: 1,
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
    pub fn cursor_position(&self) -> CursorPosition {
        let line = self.text.char_to_line(self.cursor);
        let line_start = self.text.line_to_char(line);
        let before_cursor = self.text.slice(line_start..self.cursor).to_string();

        CursorPosition {
            line,
            column: before_cursor.graphemes(true).count(),
            visual_column: UnicodeWidthStr::width(before_cursor.as_str()),
        }
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.state != self.saved_state
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

        self.record_edit(self.cursor, self.cursor, text);
    }

    pub fn backspace(&mut self) {
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
        match motion {
            Motion::Left => self.cursor = self.cursor_left(),
            Motion::Right => self.cursor = self.cursor_right(),
            Motion::Up => {
                self.move_vertically(-1);
                return;
            }
            Motion::Down => {
                self.move_vertically(1);
                return;
            }
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

        self.preferred_visual_column = None;
    }

    pub fn undo(&mut self) {
        let Some(edit) = self.undo.pop_back() else {
            return;
        };

        let inserted_end = edit.start + edit.inserted.chars().count();
        self.text.remove(edit.start..inserted_end);
        self.text.insert(edit.start, &edit.removed);
        self.cursor = edit.cursor_before;
        self.state = edit.state_before;
        self.preferred_visual_column = None;
        self.redo.push(edit);
    }

    pub fn redo(&mut self) {
        let Some(edit) = self.redo.pop() else {
            return;
        };

        let removed_end = edit.start + edit.removed.chars().count();
        self.text.remove(edit.start..removed_end);
        self.text.insert(edit.start, &edit.inserted);
        self.cursor = edit.cursor_after;
        self.state = edit.state_after;
        self.preferred_visual_column = None;
        self.push_undo(edit);
    }

    fn record_edit(&mut self, start: usize, end: usize, inserted: &str) {
        let removed = self.text.slice(start..end).to_string();
        let cursor_before = self.cursor;
        let cursor_after = start + inserted.chars().count();
        let state_before = self.state;
        let state_after = self.next_state;
        self.next_state = self.next_state.wrapping_add(1);

        self.text.remove(start..end);
        self.text.insert(start, inserted);
        self.cursor = cursor_after;
        self.state = state_after;
        self.preferred_visual_column = None;
        self.redo.clear();
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
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(edit);
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
        let text = self.text.to_string();
        let cursor_byte = self.text.char_to_byte(self.cursor);
        text.unicode_word_indices()
            .map(|(byte, _)| byte)
            .take_while(|byte| *byte < cursor_byte)
            .last()
            .map_or(0, |byte| text[..byte].chars().count())
    }

    fn word_right(&self) -> usize {
        let text = self.text.to_string();
        let cursor_byte = self.text.char_to_byte(self.cursor);
        text.unicode_word_indices()
            .map(|(byte, _)| byte)
            .find(|byte| *byte > cursor_byte)
            .map_or_else(
                || self.text.len_chars(),
                |byte| text[..byte].chars().count(),
            )
    }

    fn previous_grapheme_boundary(&self, line_start: usize, cursor: usize) -> usize {
        let prefix = self.text.slice(line_start..cursor).to_string();
        prefix
            .grapheme_indices(true)
            .next_back()
            .map_or(line_start, |(byte, _)| {
                line_start + prefix[..byte].chars().count()
            })
    }

    fn next_grapheme_boundary(&self, line: usize, cursor: usize) -> usize {
        let line_start = self.text.line_to_char(line);
        let content = self.line_without_ending(line);
        let char_offset = cursor - line_start;
        let byte_offset = byte_offset_at_char(&content, char_offset);
        content[byte_offset..]
            .graphemes(true)
            .next()
            .map_or(cursor, |grapheme| cursor + grapheme.chars().count())
    }

    fn line_content_end(&self, line: usize) -> usize {
        self.text.line_to_char(line) + self.line_without_ending(line).chars().count()
    }

    fn line_without_ending(&self, line: usize) -> String {
        self.text
            .line(line)
            .to_string()
            .trim_end_matches([
                '\r', '\n', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}',
            ])
            .to_owned()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new("")
    }
}

fn byte_offset_at_char(text: &str, char_offset: usize) -> usize {
    text.char_indices()
        .map(|(byte, _)| byte)
        .nth(char_offset)
        .unwrap_or(text.len())
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
    use super::{CursorPosition, Document, Motion};

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
}
