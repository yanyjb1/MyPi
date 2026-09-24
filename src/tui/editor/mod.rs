//! Input editor domain — the text field state machine and its satellites.
//
// - `mod.rs` (this file): the Editor state machine (insert/delete/cursor
//   moves) — see the Editor docs below for the historical notes;
// - `undo`: snapshot-based undo/redo stack;
// - `paste`: bracketed-paste markers, the paste store, collapse policy;
// - `history`: ↑↓ input-history browsing (pi-compatible semantics).

mod history;
mod paste;
mod undo;

pub use history::History;
pub use paste::PasteStore;
pub use undo::EditKind;

// Input editor — cursor model + editing actions + undo stack.
//
// `Vec<char>` instead of `String` + byte indexes:
// a CJK character is 3 bytes, and byte slicing can land mid-character and panic.
// With `Vec<char>`, index = character number is safe by construction.
//
// Cursor semantics: `cursor` is an **insertion point**, range `0..=chars.len()`.
// `cursor == 0` means before the first character, `cursor == len` after the last.
//
// Undo integration: **every method that mutates text snapshots first**
// (`self.checkpoint(kind)`), so callers never have to remember to save first —
// forgetting is the most common bug for this kind of feature.
//
// ## Paste markers are atomic
//
// Large pastes fold into `[paste #1 +30 lines]` (see `paste.rs`).
// The marker behaves as **one unit** in the editor: a single backspace removes
// the whole thing, one arrow key crosses it, Home/End treat it as indivisible.
// Implementation: before each operation, `paste::expand_over_markers` expands the
// range to cover whole markers; positions are never stored, so there is no

use std::time::Instant;

use undo::{Snapshot, UndoStack};

use crate::tui::text;

// Word-motion separators: whitespace plus common CJK/Latin punctuation.
//
// The requirement was "skip by specific separators"; these cover the common cases,
// and new ones just join either set.
const SEPARATORS: &[char] = &[
    // whitespace
    ' ', '\t', '\n', '\r', // Latin punctuation
    '.', ',', ';', ':', '!', '?', '\'', '"', '(', ')', '[', ']', '{', '}', '<', '>', '/', '\\',
    '|', '-', '_', '=', '+', '*', '&', '%', '$', '#', '@', '~', '`', '^', // CJK punctuation
    '，', '。', '、', '；', '：', '？', '！', '“', '”', '‘', '’', '（', '）', '【', '】', '《',
    '》', '「', '」', '『', '』', '—', '…', '·', '～', '·',
];

fn is_separator(c: char) -> bool {
    SEPARATORS.contains(&c)
}

// The footprint of one editing action on editor state.
//
// Exists because `apply` in app.rs has 30+ actions, of which twenty-odd only do
// "call one editor method -> one or two epilogue lines". The epilogue depends only on the footprint:
//
// - cursor-only motion -> clear the ↑↓ target-column memory and refresh completions;
// - content mutation -> additionally leave history-browse mode.
//
// This mapping used to be hand-copied per match arm (`refresh_completions()`
// duplicated 16 times, `reset_goal_col` 8): every new action needed two or three
// remembered lines, and omitting one compiled fine but misbehaved. The type system now enforces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    // Nothing happened (no-op, or finished elsewhere).
    Nothing,
    // Cursor moved, content unchanged.
    Motion,
    // Visual-line motion (↑ / ↓).
    //
    // Kept separate from `Motion` on purpose: ↑↓ maintain their own target-column
    // memory (`Editor::up/down` set `goal_col` to `Some(want)`), while the `Motion`
    // epilogue clears it. Merged, the column just recorded by an ↑↓ move would be
    // wiped immediately, and long -> short -> long line could never
    // return to the original column.
    VerticalMotion,
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

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
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
            while i > 0 && is_separator(self.chars[i - 1]) {
                i -= 1;
            }
            while i > 0 && !is_separator(self.chars[i - 1]) {
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
        while end < self.chars.len() && !is_separator(self.chars[end]) {
            end += 1;
        }
        while end < self.chars.len() && is_separator(self.chars[end]) && self.chars[end] != '\n' {
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
        // 1. swallow whitespace to the left
        while self.cursor > 0 && is_separator(self.chars[self.cursor - 1]) {
            // Separators include `\n`; whitespace and punctuation are
            // skipped together to the word boundary
            self.cursor -= 1;
        }
        // 2. swallow a whole word run
        while self.cursor > 0 && !is_separator(self.chars[self.cursor - 1]) {
            self.cursor -= 1;
        }
        self.note_cursor_move();
        Effect::Motion
    }

    // Ctrl+Right: skip a run of non-separators, then a run of separators.
    pub fn word_right(&mut self) -> Effect {
        let len = self.chars.len();
        while self.cursor < len && !is_separator(self.chars[self.cursor]) {
            self.cursor += 1;
        }
        while self.cursor < len && is_separator(self.chars[self.cursor]) {
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
    pub fn reset_goal_col(goal_col: &mut Option<usize>) {
        *goal_col = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace_with_cjk() {
        let mut e = Editor::new();
        e.insert_str("中文abc");
        assert_eq!(e.text(), "中文abc");
        assert_eq!(e.len(), 5); // 5 个字符，不是 3+9 字节
        e.backspace();
        assert_eq!(e.text(), "中文ab");
        e.backspace();
        e.backspace();
        assert_eq!(e.text(), "中文");
        e.backspace();
        assert_eq!(e.text(), "中"); // 删掉整个全角字符，不会 panic
    }

    #[test]
    fn insert_at_cursor_not_always_end() {
        let mut e = Editor::from_text("ac");
        e.home();
        e.right(); // 光标在 a|c
        e.insert_char('b');
        assert_eq!(e.text(), "abc");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn delete_removes_at_cursor() {
        let mut e = Editor::from_text("abc");
        e.home();
        e.delete();
        assert_eq!(e.text(), "bc");
        e.end();
        e.delete(); // 末尾删除无效，不 panic
        assert_eq!(e.text(), "bc");
    }

    #[test]
    fn multibyte_paste_normalizes_newlines() {
        let mut e = Editor::new();
        e.insert_str("a\r\nb\rc");
        assert_eq!(e.text(), "a\nb\nc"); // CRLF 与 CR 都归成 LF
    }

    #[test]
    fn paste_multiline_does_not_send() {
        // Paste is just an insertion; the cursor ends at the end and the
        // editor has no "send" concept
        let mut e = Editor::new();
        e.insert_str("第一行\n第二行\n第三行");
        assert_eq!(e.text().lines().count(), 3);
        assert_eq!(e.cursor(), e.len());
    }

    #[test]
    fn word_left_right_ascii() {
        let mut e = Editor::from_text("foo bar baz");
        e.end();
        e.word_left();
        assert_eq!(e.cursor(), 8); // baz 之前
        e.word_left();
        assert_eq!(e.cursor(), 4); // bar 之前
        e.word_right();
        assert_eq!(e.cursor(), 8); // 跳过 bar 及其后空格
    }

    #[test]
    fn word_left_stops_at_chinese_punctuation() {
        let mut e = Editor::from_text("你好，世界");
        e.end();
        e.word_left();
        // Cursor lands before the last CJK char: after the 3 characters
        // before it (String byte indexes would be wrong; a CJK char is 3
        // bytes)
        assert_eq!(
            e.text().chars().take(e.cursor()).collect::<String>(),
            "你好，"
        );
        e.word_left();
        // Skips the whole CJK word, reaching the start
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn word_right_skips_multiple_punctuation() {
        let mut e = Editor::from_text("a   ，。b");
        e.home();
        e.word_right();
        // After skipping "a", consecutive separators are skipped too,
        // stopping before b
        assert_eq!(e.cursor(), 6);
    }

    #[test]
    fn home_end_are_line_local() {
        let mut e = Editor::from_text("ab\ncde\nf");
        e.cursor = 4; // 在第二行的 'd' 之前
        e.line_home();
        assert_eq!(e.cursor(), 3);
        e.line_end();
        assert_eq!(e.cursor(), 6); // "cde" 之后，\n 之前
    }

    #[test]
    fn up_down_keep_goal_column() {
        // Width 4 -> "abcdef" wraps to ["abcd","ef"]
        let mut e = Editor::from_text("abcdef");
        let w = text::wrap(&e.text(), 4);
        let mut goal = None;
        e.cursor = 3; // 第 0 行第 3 格
        e.down(&w, &mut goal);
        // The next row has only 2 cells; lands at its end (= end of text)
        assert_eq!(e.cursor(), 6);
        e.up(&w, &mut goal);
        // Returns to column 3, not the clamped column of the short row
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn up_at_first_row_goes_to_start() {
        let mut e = Editor::from_text("abcdef");
        let w = text::wrap(&e.text(), 4);
        let mut goal = None;
        e.cursor = 2;
        e.up(&w, &mut goal);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn down_at_last_row_goes_to_end() {
        let mut e = Editor::from_text("abcdef");
        let w = text::wrap(&e.text(), 4);
        let mut goal = None;
        e.cursor = 5; // 第 1 行
        e.down(&w, &mut goal);
        assert_eq!(e.cursor(), 6);
    }

    #[test]
    fn goal_col_cleared_on_horizontal_move() {
        let mut goal = Some(5);
        Editor::reset_goal_col(&mut goal);
        assert_eq!(goal, None);
    }

    // ---- fast deletion ----

    #[test]
    fn delete_word_backward_matches_readline() {
        // readline's backward-kill-word deletes "word + its left
        // whitespace", exactly the range Ctrl+Left skips. One press at
        // the end therefore leaves the space before the word.
        let mut e = Editor::from_text("foo bar baz");
        e.end(); // "foo bar baz|"
        e.delete_word_backward();
        assert_eq!(e.text(), "foo bar "); // 删 "baz"，保留分隔空格
        e.delete_word_backward(); // "foo bar |"
        assert_eq!(e.text(), "foo "); // 删 "bar" + 其左侧空格
        e.delete_word_backward(); // "foo |"
        assert_eq!(e.text(), "");
        assert_eq!(e.cursor(), 0);
        e.delete_word_backward(); // 空文本上再删不 panic
        assert_eq!(e.text(), "");
    }

    #[test]
    fn delete_word_backward_from_middle() {
        // "foo bar| baz": no whitespace left of "bar" to swallow, so
        // only "bar" goes and both spaces remain (readline-compatible).
        let mut e = Editor::from_text("foo bar baz");
        e.cursor = 7; // "foo bar| baz"
        e.delete_word_backward();
        assert_eq!(e.text(), "foo  baz");
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn delete_word_backward_after_trailing_space() {
        // Cursor right after a space: "foo bar |" -> swallow the space,
        // then the word
        let mut e = Editor::from_text("foo bar baz");
        e.cursor = 8; // "foo bar |baz"
        e.delete_word_backward();
        assert_eq!(e.text(), "foo baz");
    }

    #[test]
    fn delete_word_backward_with_chinese() {
        let mut e = Editor::from_text("你好，世界");
        e.end();
        e.delete_word_backward();
        assert_eq!(e.text(), "你好，");
        e.delete_word_backward();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn delete_word_forward_stops_at_newline() {
        let mut e = Editor::from_text("aa bb\ncc");
        e.cursor = 0;
        e.delete_word_forward();
        // "aa" plus its trailing space, but never the newline
        assert_eq!(e.text(), "bb\ncc");
        e.delete_word_forward();
        assert_eq!(e.text(), "\ncc");
        e.delete_word_forward(); // 落在 \n 上，不删换行
        assert_eq!(e.text(), "\ncc");
    }

    #[test]
    fn delete_to_line_start_and_end() {
        let mut e = Editor::from_text("hello world");
        e.cursor = 5;
        e.delete_to_line_start();
        assert_eq!(e.text(), " world");
        assert_eq!(e.cursor(), 0);
        e.delete_to_line_end();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn delete_to_line_start_is_line_local() {
        // Indexes: a0 b1 \n2 c3 d4 e5 f6 \n7 g8 h9
        let mut e = Editor::from_text("ab\ncdef\ngh");
        e.cursor = 6; // "ab\ncde|f\ngh"
        e.delete_to_line_start();
        assert_eq!(e.text(), "ab\nf\ngh"); // 只删 "cde"，保留第一行与换行
        assert_eq!(e.cursor(), 3);
        e.delete_to_line_end();
        assert_eq!(e.text(), "ab\n\ngh"); // 保留两个 \n
    }

    // ---- undo / redo ----

    #[test]
    fn undo_restores_text_and_cursor() {
        let mut e = Editor::new();
        e.insert_str("hello");
        assert_eq!(e.text(), "hello");
        assert!(e.can_undo());
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert_eq!(e.cursor(), 0);
        // Nothing further to undo
        assert!(!e.undo());
    }

    #[test]
    fn redo_brings_it_back() {
        let mut e = Editor::new();
        e.insert_str("hello");
        e.undo();
        assert!(e.can_redo());
        assert!(e.redo());
        assert_eq!(e.text(), "hello");
        assert_eq!(e.cursor(), 5);
        assert!(!e.redo());
    }

    #[test]
    fn typing_is_one_undo_unit() {
        // Consecutive single-char inserts merge: Ctrl+Z reverts the
        // whole word at once
        let mut e = Editor::new();
        for c in "hello".chars() {
            e.insert_char(c);
        }
        assert_eq!(e.text(), "hello");
        assert!(e.undo());
        assert_eq!(e.text(), "", "连续打字应合并成一次撤销");
    }

    #[test]
    fn cursor_move_breaks_undo_coalescing() {
        let mut e = Editor::new();
        for c in "abc".chars() {
            e.insert_char(c);
        }
        e.left(); // 移动光标 → 切断合并
        e.insert_char('X');
        assert_eq!(e.text(), "abXc");
        // The first undo removes the X only
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
        // The second removes "abc"
        assert!(e.undo());
        assert_eq!(e.text(), "");
    }

    #[test]
    fn paste_is_its_own_undo_unit() {
        let mut e = Editor::new();
        e.insert_str("a\nb\nc");
        assert!(e.undo());
        assert_eq!(e.text(), "", "整块粘贴一次撤销干净");
    }

    #[test]
    fn undo_after_quick_deletes() {
        // Fast deletions must be undoable — the core purpose of the
        // safety net
        let mut e = Editor::from_text("one two three");
        e.end();
        e.delete_word_backward();
        assert_eq!(e.text(), "one two ");
        assert!(e.undo());
        assert_eq!(e.text(), "one two three");

        e.delete_to_line_start();
        assert_eq!(e.text(), "");
        assert!(e.undo());
        assert_eq!(e.text(), "one two three");
    }

    #[test]
    fn noop_delete_does_not_push_undo_step() {
        // Ctrl+U on an empty line deletes nothing and must not consume
        // an undo step
        let mut e = Editor::from_text("abc");
        e.home();
        e.delete_to_line_start();
        assert!(!e.can_undo(), "无效果的删除不该产生撤销步骤");
    }

    #[test]
    fn new_edit_after_undo_clears_redo() {
        let mut e = Editor::new();
        e.insert_str("hello");
        e.undo();
        assert!(e.can_redo());
        e.insert_char('X');
        assert!(!e.can_redo(), "撤销后新增内容会让重做链失效");
    }

    #[test]
    fn replace_range_swaps_text_and_is_undoable() {
        // Completion scenario: "src/ma" -> replace "ma" with "main.rs"
        let mut e = Editor::from_text("src/ma");
        e.replace_range(4, 6, "main.rs");
        assert_eq!(e.text(), "src/main.rs");
        assert_eq!(e.cursor(), 11);
        assert!(e.undo());
        assert_eq!(e.text(), "src/ma");
    }

    #[test]
    fn replace_range_clamps_out_of_bounds() {
        let mut e = Editor::from_text("abc");
        e.replace_range(1, 999, "X");
        assert_eq!(e.text(), "aX");
        e.replace_range(999, 999, "Y");
        assert_eq!(e.text(), "aXY");
    }

    #[test]
    fn clear_wipes_undo_history() {
        let mut e = Editor::new();
        e.insert_str("hello");
        e.clear();
        assert!(!e.can_undo(), "提交后应重置撤销历史");
        assert!(!e.can_redo());
    }

    // ---- paste marker atomicity (requirement: deleting any part removes the whole) ----

    // 35 lines -> must fold.
    fn big_paste() -> String {
        (0..35)
            .map(|i| format!("code line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn large_paste_collapses_to_marker() {
        let mut e = Editor::new();
        assert!(e.insert_paste(&big_paste()));
        let t = e.text();
        assert!(t.starts_with("[paste #1 +35 lines]"), "实际: {t}");
        assert_eq!(t.lines().count(), 1, "折成一行的标记");
    }

    #[test]
    fn small_paste_is_inserted_verbatim() {
        let mut e = Editor::new();
        assert!(!e.insert_paste("只有两行\n第二行"));
        assert_eq!(e.text(), "只有两行\n第二行");
    }

    #[test]
    fn backspace_right_after_marker_removes_whole_marker() {
        // The core required behavior: cursor at the marker right edge,
        // one backspace removes it all
        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        assert!(e.text().contains("[paste #1"));
        e.backspace();
        // The marker is gone; the trailing space added at insertion
        // stays (harmless, submit trims it)
        assert_eq!(e.text(), " ", "标记必须整块删掉，不能只删最后一个字符");
    }

    #[test]
    fn backspace_inside_marker_removes_whole_marker() {
        // Another described case: cursor inside the marker
        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        e.cursor = 10; // 标记内部
        e.backspace();
        assert!(!e.text().contains("[paste"), "从内部退格也要整块消失");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn delete_forward_on_marker_removes_whole_marker() {
        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        e.cursor = 0;
        e.delete();
        assert!(!e.text().contains("[paste"), "整块消失");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn ctrl_w_removes_marker_atomically() {
        // With a space before the marker, Ctrl+Backspace should swallow
        // the marker in one stroke
        let mut e = Editor::new();
        e.insert_str("看看这段 ");
        e.insert_paste(&big_paste());
        e.end();
        e.delete_word_backward();
        assert!(
            !e.text().contains("[paste"),
            "Ctrl+W 必须吃掉整个标记，实际: {}",
            e.text()
        );
    }

    #[test]
    fn ctrl_u_removes_marker_atomically() {
        let mut e = Editor::new();
        e.insert_str("前缀 ");
        e.insert_paste(&big_paste());
        e.end();
        e.delete_to_line_start();
        // Delete-to-start includes the trailing space, so the line
        // empties completely
        assert_eq!(e.text(), "", "Ctrl+U 删到行首应连标记一起干净删掉");
    }

    #[test]
    fn partial_range_delete_absorbs_marker() {
        // Even a range touching only part of a marker must remove the
        // whole marker
        let mut e = Editor::new();
        e.insert_str("xx");
        e.insert_paste(&big_paste());
        e.insert_str("yy");
        // Delete from the first character into the middle of the marker
        let text = e.text();
        let marker_start = text.find("[paste").unwrap();
        let from = 1usize;
        let to = marker_start + 5; // 切进标记内部
        e.replace_range(from, to, "");
        assert!(
            !e.text().contains("[paste"),
            "部分覆盖也要整块删除，实际: {}",
            e.text()
        );
    }

    #[test]
    fn arrows_jump_over_marker() {
        let mut e = Editor::new();
        e.insert_str("a");
        e.insert_paste(&big_paste());
        e.insert_str("b");
        // Text: "a [paste #1 +35 lines] b"
        //          ^                  ^
        //          1                  22
        let start = e.text().find("[paste").unwrap();
        let end = start + "[paste #1 +35 lines]".chars().count();

        e.cursor = start; // 标记左端
        e.right();
        assert_eq!(e.cursor(), end, "右移应一步跨过整个标记");

        e.left();
        assert_eq!(e.cursor(), start, "左移应一步跨回标记左端");
    }

    #[test]
    fn a_seam_between_two_markers_deletes_the_expected_side() {
        // Two adjacent markers with the cursor exactly at the seam: the
        // character before the cursor belongs to the left marker, the one
        // under it to the right marker. The old loose predicate matched the
        // left marker for **both** directions, so forward Delete removed the
        // marker behind the cursor.
        let text = "[paste #1 +35 lines][paste #2 +40 lines]";
        let seam = "[paste #1 +35 lines]".chars().count();

        let mut e = Editor::from_text(text);
        e.cursor = seam;
        assert_eq!(e.delete(), Effect::Content);
        assert_eq!(e.text(), "[paste #1 +35 lines]", "Delete 必须删右边那个");

        let mut e = Editor::from_text(text);
        e.cursor = seam;
        assert_eq!(e.backspace(), Effect::Content);
        assert_eq!(e.text(), "[paste #2 +40 lines]", "Backspace 必须删左边那个");
    }

    #[test]
    fn marker_edits_report_their_footprint() {
        // `Effect` decides the epilogue (leave history-browse mode, refresh
        // completions, drop the ↑↓ target column). The marker branches used to
        // report `Nothing` even though they had deleted content or moved the
        // cursor, silently skipping all of it.
        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        e.end();
        assert_eq!(e.backspace(), Effect::Content, "删掉标记是内容变更");

        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        e.cursor = 0;
        assert_eq!(e.delete(), Effect::Content, "正向删掉标记也是内容变更");

        let mut e = Editor::new();
        e.insert_str("a");
        e.insert_paste(&big_paste());
        let start = e.text().find("[paste").unwrap();
        e.cursor = start;
        assert_eq!(e.right(), Effect::Motion, "跨过标记是光标移动");
        assert_eq!(e.left(), Effect::Motion, "跨回标记也是光标移动");
    }

    #[test]
    fn undo_restores_deleted_marker() {
        // After undo the marker must return and still expand on submit
        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        let before = e.text();
        e.backspace();
        assert!(!e.text().contains("[paste"));
        assert!(e.undo());
        assert_eq!(e.text(), before, "撤销应恢复标记");
        // Key point: the restored marker still expands (its number was
        // not reused)
        assert!(e.expanded_text().contains("code line 34"));
    }

    #[test]
    fn expanded_text_restores_original() {
        let mut e = Editor::new();
        let src = big_paste();
        e.insert_paste(&src);
        e.insert_str(" 帮我看看");
        let out = e.expanded_text();
        assert!(out.starts_with("code line 0"), "提交内容应是原文");
        assert!(out.ends_with(" 帮我看看"));
        assert_eq!(out.matches("code line").count(), 35);
    }

    #[test]
    fn marker_gets_surrounding_spaces() {
        // Folding leaves a space on each side so Ctrl+Backspace naturally
        // swallows it. The left side depends on the preceding character;
        // the right side is added unconditionally (insertion cannot know
        // what comes next).
        let mut e = Editor::new();
        e.insert_str("看看");
        e.insert_paste(&big_paste());
        let t = e.text();
        assert!(t.starts_with("看看 [paste"), "左侧应补空格: {t}");
        assert!(t.ends_with("] "), "右侧应留空格: {t}");

        // Not added again when already at line start / after whitespace
        let mut e2 = Editor::new();
        e2.insert_paste(&big_paste());
        assert!(!e2.text().starts_with(' '), "行首不该多一个空格");
    }

    #[test]
    fn trailing_space_makes_ctrl_backspace_work() {
        // Direct encoding of the requirement: paste then Ctrl+Backspace
        // deletes the whole thing
        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        e.end();
        e.delete_word_backward(); // 先吃掉尾随空格 + 标记
        assert_eq!(e.text(), "", "实际: {}", e.text());
    }

    #[test]
    fn multiple_markers_get_distinct_ids() {
        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        e.insert_str(" 中间 ");
        e.insert_paste(&big_paste());
        let t = e.text();
        assert!(t.contains("[paste #1"), "{t}");
        assert!(t.contains("[paste #2"), "第二个标记必须换编号: {t}");
        // Both expand
        assert_eq!(e.expanded_text().matches("code line 0").count(), 2);
    }

    #[test]
    fn clearing_editor_drops_paste_content() {
        let mut e = Editor::new();
        e.insert_paste(&big_paste());
        e.clear();
        assert!(e.pastes.is_empty(), "提交后应释放折叠内容占的内存");
        assert_eq!(e.expanded_text(), "");
    }
}
