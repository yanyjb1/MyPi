//! The editor: text as `Vec<char>`, an insertion-point cursor, undo, paste markers.

use std::time::Instant;

mod history;
mod paste;
mod separators;
mod undo;

pub use history::History;
use paste::PasteStore;
use separators::SeparatorTable;
use undo::{EditKind, Snapshot, UndoStack};

use crate::ui::tui::text;

// What one action did to the editor state; the caller's epilogue reads only this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    // Nothing happened (no-op, or finished elsewhere).
    Nothing,
    // Cursor moved, content unchanged.
    Motion,
    // Content was mutated.
    Content,
}

// Input editor state.
#[derive(Debug, Clone, Default)]
pub struct Editor {
    chars: Vec<char>,
    cursor: usize,
    // Undo/redo stack. See `undo.rs` for the merge strategy.
    undo: UndoStack,
    // Folded large pastes. Used to restore the full text on submit.
    pastes: PasteStore,
    // The block boundary set, shared with the completion trigger.
    pub separators: SeparatorTable,
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    // Build from existing text, cursor at the end.
    pub fn from_text(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        Self {
            chars,
            cursor,
            undo: UndoStack::new(),
            pastes: PasteStore::new(),
            separators: SeparatorTable::default(),
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
        self.undo.clear();
        self.pastes.clear();
    }

    // ---- paste markers ----

    // The text as displayed (markers **not** expanded). Used for rendering.
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    // The text sent to the model: every marker restored to its original content.
    pub fn expanded_text(&self) -> String {
        self.pastes.expand(&self.text())
    }

    // Insert a potentially huge paste.
    //
    // Above the threshold (>10 lines or >1000 chars) it folds into a marker;
    // otherwise the raw text is inserted. Returns whether folding happened.
    pub fn insert_paste(&mut self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
        if !paste::should_collapse(&normalized) {
            self.insert_str(&normalized);
            return false;
        }

        let line_count = normalized.split('\n').count();
        let char_count = normalized.chars().count();
        let id = self.pastes.add(normalized);
        let marker = PasteStore::marker_text(id, line_count, char_count);

        // A space on each side of the marker (when adjacent to word
        // characters) lets Ctrl+Backspace swallow it in one stroke —
        // "naturally deletable" was the requirement. The left space is
        // decided by the preceding character; the right one **must be
        // added unconditionally**: at insertion time nothing follows, and
        // there is no way to know what will. A stray trailing space is
        // harmless (submit trims it); missing it removes the fast path.
        let need_lead = self
            .chars
            .get(self.cursor.wrapping_sub(1))
            .is_some_and(|c| !c.is_whitespace());

        self.checkpoint(EditKind::Other);
        if need_lead {
            self.insert_char_raw(' ');
        }
        for c in marker.chars() {
            self.insert_char_raw(c);
        }
        self.insert_char_raw(' ');
        true
    }

    // The marker a **backward** action acts on at `pos`: the one holding the
    // character to the left — strictly inside it, or exactly at its right edge.
    // At the seam between two adjacent markers that is the left-hand one.
    fn marker_left(&self, pos: usize) -> Option<paste::Marker> {
        paste::marker_spanning(&self.chars, pos, false, true)
    }

    // The marker a **forward** action acts on at `pos`: the one holding the
    // character under the cursor — strictly inside it, or exactly at its left
    // edge. At the seam between two adjacent markers that is the right-hand
    // one (the old loose predicate matched the left-hand one, so forward
    // delete removed the marker *before* the cursor).
    fn marker_right(&self, pos: usize) -> Option<paste::Marker> {
        paste::marker_spanning(&self.chars, pos, true, false)
    }

    // Expand `[start, end)` to cover every marker it touches.
    fn absorb_markers(&self, start: usize, end: usize) -> (usize, usize) {
        paste::expand_over_markers(&self.chars, start, end)
    }

    // ---- undo / redo ----

    // Snapshot **before** mutating text.
    //
    // `Typing` merges consecutive character inserts into one undo unit; everything else is its own step.
    fn checkpoint(&mut self, kind: EditKind) {
        let before = Snapshot {
            chars: self.chars.clone(),
            cursor: self.cursor,
        };
        self.undo.record(before, kind, Instant::now());
    }

    // Call after cursor movement: breaks undo merging.
    //
    // Without it, "type two words -> move the cursor -> type more"
    // merges into one undo and Ctrl+Z reverts the earlier word too.
    pub fn note_cursor_move(&mut self) {
        self.undo.break_coalescing();
    }

    // Undo one step. Returns false when there is no history.
    pub fn undo(&mut self) -> bool {
        let current = Snapshot {
            chars: self.chars.clone(),
            cursor: self.cursor,
        };
        match self.undo.undo(current) {
            Some(s) => {
                self.chars = s.chars;
                self.cursor = s.cursor.min(self.chars.len());
                true
            }
            None => false,
        }
    }

    // Redo one step.
    pub fn redo(&mut self) -> bool {
        let current = Snapshot {
            chars: self.chars.clone(),
            cursor: self.cursor,
        };
        match self.undo.redo(current) {
            Some(s) => {
                self.chars = s.chars;
                self.cursor = s.cursor.min(self.chars.len());
                true
            }
            None => false,
        }
    }

    // ---- editing ----

    // Insert one character at the cursor and move right.
    pub fn insert_char(&mut self, c: char) -> Effect {
        self.checkpoint(EditKind::Typing);
        self.insert_char_raw(c);
        Effect::Content
    }

    // Insert without an undo record. Batch operations use this to keep the undo stack lean.
    fn insert_char_raw(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    // Insert a text run at the cursor (paste goes through here).
    // Normalizes internal `\r\n` / `\r` to `\n`, avoiding stray empty lines.
    pub fn insert_str(&mut self, s: &str) -> Effect {
        if s.is_empty() {
            return Effect::Nothing;
        }
        // A whole paste is one undo step and never merges with later typing.
        self.checkpoint(EditKind::Other);
        let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
        for c in normalized.chars() {
            self.insert_char_raw(c);
        }
        Effect::Content
    }

    // Replace the `from..to` character range with `text` (used by completion).
    //
    // Out-of-range indexes are clamped safely; never panics.
    pub fn replace_range(&mut self, from: usize, to: usize, text: &str) -> Effect {
        let len = self.chars.len();
        let from = from.min(len);
        let to = to.min(len).max(from);
        // Real deletions (non-empty range) must also absorb touched markers, or half markers survive
        let (from, to) = if from < to {
            self.absorb_markers(from, to)
        } else {
            (from, to)
        };
        self.checkpoint(EditKind::Other);
        self.chars.drain(from..to);
        self.cursor = from;
        for c in text.chars() {
            self.insert_char_raw(c);
        }
        Effect::Content
    }

    // Delete one character before the cursor (Backspace).
    //
    // If the cursor sits at the right edge of a paste marker, the
    // **whole marker goes away** — "delete one char inside, the entire
    // thing disappears" was the requirement.
    pub fn backspace(&mut self) -> Effect {
        if self.cursor == 0 {
            return Effect::Nothing;
        }
        // First check whether the cursor hugs a marker on its left (cursor at marker end or inside)
        //
        // The second lookup covers the character right after the marker (the
        // trailing space `insert_paste` adds): one press still removes the
        // whole marker rather than eating that space first.
        if let Some(m) = self
            .marker_left(self.cursor)
            .or_else(|| self.marker_left(self.cursor - 1))
        {
            self.checkpoint(EditKind::Other);
            self.chars.drain(m.start..m.end);
            self.cursor = m.start;
            // Content, not Nothing: the epilogue (leave history-browse mode,
            // refresh completions, drop goal_col) keys off the footprint.
            return Effect::Content;
        }
        self.checkpoint(EditKind::Other);
        self.cursor -= 1;
        self.chars.remove(self.cursor);
        Effect::Content
    }

    // Delete the character at the cursor (Delete).
    //
    // If the cursor is on a paste marker (left edge included), delete the whole marker.
    pub fn delete(&mut self) -> Effect {
        if self.cursor >= self.chars.len() {
            return Effect::Nothing;
        }
        if let Some(m) = self.marker_right(self.cursor) {
            self.checkpoint(EditKind::Other);
            self.chars.drain(m.start..m.end);
            self.cursor = m.start;
            return Effect::Content; // see backspace: the footprint drives the epilogue
        }
        self.checkpoint(EditKind::Other);
        self.chars.remove(self.cursor);
        Effect::Content
    }

    // ---- fast deletion (Ctrl+W / Alt+D / Ctrl+U / Ctrl+K) ----

    // Delete to the left: swallow adjacent whitespace first, then a run of non-separators.
    //
    // Same walk as `word_left`, except the traversed characters are deleted.
    // Whitespace first so that on `foo bar|` one press removes `bar`,
    // not just the space.
    pub fn delete_word_backward(&mut self) -> Effect {
        let start = {
            let mut i = self.cursor;
            // Swallow the boundary first, then the whole block.
            while i > 0 && self.separators.is_hard(self.chars[i - 1]) {
                i -= 1;
            }
            while i > 0 && !self.separators.is_hard(self.chars[i - 1]) {
                i -= 1;
            }
            i
        };
        if start == self.cursor {
            return Effect::Nothing; // nothing deleted: no pointless undo step
        }
        // Expand to cover touched markers: better to delete a whole marker than leave half
        let (lo, _) = self.absorb_markers(start, self.cursor);
        self.checkpoint(EditKind::Other);
        self.chars.drain(lo..self.cursor);
        self.cursor = lo;
        Effect::Content
    }

    // Delete to the right: swallow a run of non-separators first, then adjacent whitespace.
    pub fn delete_word_forward(&mut self) -> Effect {
        let mut end = self.cursor;
        while end < self.chars.len() && !self.separators.is_hard(self.chars[end]) {
            end += 1;
        }
        // Swallow the trailing boundary, never a newline.
        while end < self.chars.len()
            && self.separators.is_hard(self.chars[end])
            && self.chars[end] != '\n'
        {
            end += 1;
        }
        if end == self.cursor {
            return Effect::Nothing;
        }
        let (_, hi) = self.absorb_markers(self.cursor, end);
        self.checkpoint(EditKind::Other);
        self.chars.drain(self.cursor..hi);
        Effect::Content
    }

    // Delete to the start of the current logical line (not the leading
    // `\n`; the previous line's newline is not eaten).
    pub fn delete_to_line_start(&mut self) -> Effect {
        let start = {
            let mut i = self.cursor;
            while i > 0 && self.chars[i - 1] != '\n' {
                i -= 1;
            }
            i
        };
        if start == self.cursor {
            return Effect::Nothing; // nothing deleted: no pointless undo step
        }
        // Expand to cover touched markers: better to delete a whole marker than leave half
        let (lo, _) = self.absorb_markers(start, self.cursor);
        self.checkpoint(EditKind::Other);
        self.chars.drain(lo..self.cursor);
        self.cursor = lo;
        Effect::Content
    }

    // Delete to the end of the current logical line (not the trailing `\n`).
    pub fn delete_to_line_end(&mut self) -> Effect {
        let mut end = self.cursor;
        while end < self.chars.len() && self.chars[end] != '\n' {
            end += 1;
        }
        if end == self.cursor {
            return Effect::Nothing;
        }
        let (_, hi) = self.absorb_markers(self.cursor, end);
        self.checkpoint(EditKind::Other);
        self.chars.drain(self.cursor..hi);
        Effect::Content
    }

    // ---- cursor movement ----

    // Move left one step. At the right edge of a marker, **jump across the whole marker**.
    pub fn left(&mut self) -> Effect {
        // Cursor at the marker right edge -> jump to the marker left edge in one step
        if let Some(m) = self.marker_left(self.cursor)
            && m.end == self.cursor
        {
            self.cursor = m.start;
            self.note_cursor_move();
            // Motion, not Nothing: the cursor did move, so the epilogue must
            // refresh completions and clear the ↑↓ target column.
            return Effect::Motion;
        }
        self.cursor = self.cursor.saturating_sub(1);
        self.note_cursor_move();
        Effect::Motion
    }

    // Move right one step. At the left edge of a marker, **jump across the whole marker**.
    pub fn right(&mut self) -> Effect {
        if let Some(m) = self.marker_right(self.cursor)
            && m.start == self.cursor
        {
            self.cursor = m.end;
            self.note_cursor_move();
            return Effect::Motion; // see left(): a real move needs the motion epilogue
        }
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
        self.note_cursor_move();
        Effect::Motion
    }

    pub fn home(&mut self) -> Effect {
        self.cursor = 0;
        self.note_cursor_move();
        Effect::Motion
    }

    pub fn end(&mut self) -> Effect {
        self.cursor = self.chars.len();
        self.note_cursor_move();
        Effect::Motion
    }

    // Move to the start of the current logical line (used when `\n`
    // exists; what Home expects).
    pub fn line_home(&mut self) -> Effect {
        while self.cursor > 0 && self.chars[self.cursor - 1] != '\n' {
            self.cursor -= 1;
        }
        self.note_cursor_move();
        Effect::Motion
    }

    // Move to the end of the current logical line.
    pub fn line_end(&mut self) -> Effect {
        while self.cursor < self.chars.len() && self.chars[self.cursor] != '\n' {
            self.cursor += 1;
        }
        self.note_cursor_move();
        Effect::Motion
    }

    // Ctrl+Left: skip whitespace leftward, then a run of non-separators or a run of separators.
    pub fn word_left(&mut self) -> Effect {
        // Step over the boundary, then the whole block.
        while self.cursor > 0 && self.separators.is_hard(self.chars[self.cursor - 1]) {
            self.cursor -= 1;
        }
        while self.cursor > 0 && !self.separators.is_hard(self.chars[self.cursor - 1]) {
            self.cursor -= 1;
        }
        self.note_cursor_move();
        Effect::Motion
    }

    // Ctrl+Right: skip a run of non-separators, then a run of separators.
    pub fn word_right(&mut self) -> Effect {
        let len = self.chars.len();
        while self.cursor < len && !self.separators.is_hard(self.chars[self.cursor]) {
            self.cursor += 1;
        }
        while self.cursor < len && self.separators.is_hard(self.chars[self.cursor]) {
            self.cursor += 1;
        }
        self.note_cursor_move();
        Effect::Motion
    }

    // ---- vertical motion: jumps between wrapped visual rows; see layout/input ----

    // Move up one visual row.
    //
    // `wrapped` must be wrapped at **the same width as rendering** — a
    // different width shifts `row` out of alignment with the view and
    // the cursor "cannot move back" to what is above.
    //
    // Only cursor movement lives here; **viewport scrolling is
    // `layout::adjust_scroll`'s job**. Deliberately separated: merged,
    // the cursor would stick at the edge while the viewport never moves.
    //
    // The caller holds `goal_col`: across consecutive ↑ presses the
    // column stays put, so long -> short -> long line returns to the
    // original column (standard editor behavior).
    pub fn up(&mut self, wrapped: &text::Wrapped, goal_col: &mut Option<usize>) -> Effect {
        let (row, col) = wrapped.locate(self.cursor);
        let want = goal_col.unwrap_or(col);
        *goal_col = Some(want);
        self.cursor = if row == 0 {
            0
        } else {
            wrapped.char_at(row - 1, want)
        };
        self.note_cursor_move();
        Effect::Motion
    }

    // Move down one visual row. Cursor only; the viewport is untouched.
    pub fn down(&mut self, wrapped: &text::Wrapped, goal_col: &mut Option<usize>) -> Effect {
        let (row, col) = wrapped.locate(self.cursor);
        let want = goal_col.unwrap_or(col);
        *goal_col = Some(want);
        let last = wrapped.len().saturating_sub(1);
        self.cursor = if row >= last {
            self.chars.len()
        } else {
            wrapped.char_at(row + 1, want)
        };
        self.note_cursor_move();
        Effect::Motion
    }

    // Horizontal motion should clear the target-column memory, or ↑↓
    // would jump back to the old column.
}
