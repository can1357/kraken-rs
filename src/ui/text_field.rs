//! Single editing primitive behind every text input in the app.
//!
//! A [`TextField`] owns the string value plus caret and selection state, and
//! implements the standard editing operations (insert over selection,
//! char/word/edge movement, select-all, forward/backward deletion). Views
//! read the value through `Deref<Target = str>` and position the caret with
//! [`TextField::caret`]; input front-ends mutate it exclusively through the
//! methods here so every field behaves identically.

use std::ops::{Deref, Range};

/// Movement / deletion granularity for caret operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Jump {
    /// One character.
    Char,
    /// One word (alt/option).
    Word,
    /// To the line edge (command / Home / End).
    Edge,
}

/// Caret geometry in character units, for renderers that estimate glyph
/// advances: `line`/`column` locate the caret, `selection` is the selected
/// char range (empty when nothing is selected).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Caret {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) selection_start: usize,
    pub(crate) selection_end: usize,
}

impl Caret {
    /// Selected char range within the whole text.
    pub(crate) fn selection(&self) -> Range<usize> {
        self.selection_start..self.selection_end
    }
}

/// Editable text state: value, caret, and selection anchor (byte offsets).
#[derive(Clone, Debug, Default)]
pub(crate) struct TextField {
    text: String,
    cursor: usize,
    anchor: usize,
}

impl Deref for TextField {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Display for TextField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl TextField {
    /// The current value.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the value, placing the caret at the end.
    pub(crate) fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.anchor = self.cursor;
    }

    /// Empties the field.
    pub(crate) fn clear(&mut self) {
        self.set_text(String::new());
    }

    /// Selected byte range (empty when nothing is selected).
    fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    /// The selected text, empty when the selection is collapsed.
    pub(crate) fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    /// Inserts text at the caret, replacing any selection.
    pub(crate) fn insert(&mut self, text: &str) {
        let range = self.selection();
        self.text.replace_range(range.clone(), text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
    }

    /// Selects the entire value.
    pub(crate) fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    /// Removes and returns the selected text (for cut).
    pub(crate) fn take_selection(&mut self) -> String {
        let range = self.selection();
        let taken = self.text[range.clone()].to_owned();
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.anchor = self.cursor;
        taken
    }

    /// Byte offset of the previous caret stop.
    fn previous_offset(&self, jump: Jump) -> usize {
        match jump {
            Jump::Char => self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index),
            Jump::Word => {
                let before = &self.text[..self.cursor];
                let mut chars = before.char_indices().rev().peekable();
                let mut offset = 0;
                let mut seen_word = false;
                for (index, character) in &mut chars {
                    if is_word(character) {
                        seen_word = true;
                        offset = index;
                    } else if seen_word {
                        offset = index + character.len_utf8();
                        break;
                    } else {
                        offset = index;
                    }
                }
                offset
            }
            Jump::Edge => self.text[..self.cursor]
                .rfind('\n')
                .map_or(0, |index| index + 1),
        }
    }

    /// Byte offset of the next caret stop.
    fn next_offset(&self, jump: Jump) -> usize {
        match jump {
            Jump::Char => self.text[self.cursor..]
                .chars()
                .next()
                .map_or(self.text.len(), |character| {
                    self.cursor + character.len_utf8()
                }),
            Jump::Word => {
                let after = &self.text[self.cursor..];
                let mut offset = after.len();
                let mut seen_word = false;
                for (index, character) in after.char_indices() {
                    if is_word(character) {
                        seen_word = true;
                    } else if seen_word {
                        offset = index;
                        break;
                    }
                }
                self.cursor + offset
            }
            Jump::Edge => self.text[self.cursor..]
                .find('\n')
                .map_or(self.text.len(), |index| self.cursor + index),
        }
    }

    /// Moves the caret left; a collapsing move without `select` lands on the
    /// selection edge like native fields.
    pub(crate) fn move_left(&mut self, jump: Jump, select: bool) {
        if !select && self.cursor != self.anchor && jump == Jump::Char {
            self.cursor = self.selection().start;
        } else {
            self.cursor = self.previous_offset(jump);
        }
        if !select {
            self.anchor = self.cursor;
        }
    }

    /// Moves the caret right; mirrors [`TextField::move_left`].
    pub(crate) fn move_right(&mut self, jump: Jump, select: bool) {
        if !select && self.cursor != self.anchor && jump == Jump::Char {
            self.cursor = self.selection().end;
        } else {
            self.cursor = self.next_offset(jump);
        }
        if !select {
            self.anchor = self.cursor;
        }
    }

    /// Moves the caret one line up (`-1`) or down (`1`) in multiline text,
    /// keeping the column when possible.
    pub(crate) fn move_vertical(&mut self, delta: i32, select: bool) {
        let (line, column) = self.line_column(self.cursor);
        let target = if delta < 0 {
            let Some(line) = line.checked_sub(1) else {
                self.cursor = 0;
                if !select {
                    self.anchor = 0;
                }
                return;
            };
            line
        } else {
            line + 1
        };
        let mut lines = self.text.split('\n');
        let mut offset = 0;
        for _ in 0..target {
            match lines.next() {
                Some(line) => offset += line.len() + 1,
                None => break,
            }
        }
        if offset > self.text.len() {
            self.cursor = self.text.len();
        } else {
            let line_text = lines.next().unwrap_or("");
            let column_bytes = line_text
                .char_indices()
                .nth(column)
                .map_or(line_text.len(), |(index, _)| index);
            self.cursor = offset + column_bytes;
        }
        if !select {
            self.anchor = self.cursor;
        }
    }

    /// Deletes backward: the selection when present, else `jump` behind the caret.
    pub(crate) fn backspace(&mut self, jump: Jump) {
        if self.cursor == self.anchor {
            self.anchor = self.previous_offset(jump);
        }
        self.take_selection();
    }

    /// Deletes forward: the selection when present, else `jump` ahead of the caret.
    pub(crate) fn delete_forward(&mut self, jump: Jump) {
        if self.cursor == self.anchor {
            self.anchor = self.next_offset(jump);
        }
        self.take_selection();
    }

    /// Line and char-column of a byte offset.
    fn line_column(&self, offset: usize) -> (usize, usize) {
        let before = &self.text[..offset];
        let line = before.matches('\n').count();
        let column = before
            .rfind('\n')
            .map_or(before, |index| &before[index + 1..])
            .chars()
            .count();
        (line, column)
    }

    /// Places the caret at a (line, char-column) hit from pointer estimates,
    /// clamping to the text.
    pub(crate) fn place_caret(&mut self, line: usize, column: usize) {
        let mut offset = 0;
        let mut lines = self.text.split('\n');
        for _ in 0..line.min(self.text.matches('\n').count()) {
            offset += lines.next().map_or(0, |line| line.len() + 1);
        }
        let line_text = lines.next().unwrap_or("");
        let column_bytes = line_text
            .char_indices()
            .nth(column)
            .map_or(line_text.len(), |(index, _)| index);
        self.cursor = offset + column_bytes;
        self.anchor = self.cursor;
    }

    /// Caret geometry in char units for rendering.
    pub(crate) fn caret(&self) -> Caret {
        let (line, column) = self.line_column(self.cursor);
        let range = self.selection();
        Caret {
            line,
            column,
            selection_start: self.text[..range.start].chars().count(),
            selection_end: self.text[..range.end].chars().count(),
        }
    }
}

/// Word characters for option/alt caret jumps.
fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str) -> TextField {
        let mut field = TextField::default();
        field.set_text(text);
        field
    }

    #[test]
    fn insert_replaces_selection() {
        let mut input = field("hello world");
        input.select_all();
        input.insert("bye");
        assert_eq!(input.text(), "bye");
        assert_eq!(input.caret().column, 3);
    }

    #[test]
    fn word_jumps_stop_at_word_boundaries() {
        let mut input = field("git log --oneline");
        input.move_left(Jump::Word, false);
        assert_eq!(input.caret().column, 10);
        input.move_left(Jump::Word, false);
        assert_eq!(input.caret().column, 4);
        input.move_right(Jump::Word, false);
        assert_eq!(input.caret().column, 7);
    }

    #[test]
    fn edge_jumps_respect_lines() {
        let mut input = field("first\nsecond line");
        input.move_left(Jump::Edge, false);
        assert_eq!(
            input.caret(),
            Caret {
                line: 1,
                column: 0,
                selection_start: 6,
                selection_end: 6,
            }
        );
        input.move_right(Jump::Edge, false);
        assert_eq!(input.caret().column, 11);
    }

    #[test]
    fn backspace_word_and_selection() {
        let mut input = field("delete this_word");
        input.backspace(Jump::Word);
        assert_eq!(input.text(), "delete ");
        input.move_left(Jump::Word, true);
        input.backspace(Jump::Char);
        assert_eq!(input.text(), "");
    }

    #[test]
    fn collapse_lands_on_selection_edge() {
        let mut input = field("abc");
        input.select_all();
        input.move_left(Jump::Char, false);
        assert_eq!(input.caret().column, 0);
        input.select_all();
        input.move_right(Jump::Char, false);
        assert_eq!(input.caret().column, 3);
    }

    #[test]
    fn vertical_moves_clamp_column() {
        let mut input = field("long first line\nab\nthird");
        input.place_caret(0, 10);
        input.move_vertical(1, false);
        assert_eq!(
            input.caret(),
            Caret {
                line: 1,
                column: 2,
                selection_start: 18,
                selection_end: 18,
            }
        );
        input.move_vertical(1, false);
        assert_eq!(input.caret().line, 2);
        // Column is not sticky across moves; it carries the clamped value.
        assert_eq!(input.caret().column, 2);
        input.move_vertical(1, false);
        assert_eq!(input.caret().column, 5, "past-last-line lands at text end");
    }

    #[test]
    fn delete_forward_at_end_is_noop() {
        let mut input = field("x");
        input.delete_forward(Jump::Char);
        assert_eq!(input.text(), "x");
    }

    #[test]
    fn multibyte_chars_stay_on_boundaries() {
        let mut input = field("héllo wörld");
        input.move_left(Jump::Word, false);
        input.backspace(Jump::Char);
        assert_eq!(input.text(), "héllowörld");
        input.select_all();
        assert_eq!(input.selected_text(), "héllowörld");
    }
}
