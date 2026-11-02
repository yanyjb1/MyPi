//! Undo / redo — mirrors pi's `UndoStack` + `EditorSnapshot`
//! （`packages/tui/src/components/editor.ts`）。
//!
//! Design trade-off (why not "a snapshot per mutation"):
//!
//! Snapshotting every inserted character would make typing "hello" push 5
//! snapshots, and Ctrl+Z would need 5 presses to revert one word — that is not
//!
//! So this implements **coalescing**: consecutive single-character inserts count
//! as one edit with a single snapshot. Any other action (deletion, newline, paste,
//! input after a cursor move) breaks the merge and forms its own undo unit.
//! This matches pi's `skipUndoCoalescing` parameter and mainstream editor behavior.
//!
//! A **time gate** is also added: inserts more than `COALESCE_WINDOW` apart break the merge.
//! Typing a word, pausing to think, then typing another yields two undo steps,
//! which matches intuition (a pause in time is a mental paragraph break).

use std::time::{Duration, Instant};

// Typing coalesce window: a longer gap splits into two independent undo units.
pub const COALESCE_WINDOW: Duration = Duration::from_millis(900);

// Undo stack capacity cap. Keeps long sessions from eating memory.
//
// One snapshot is roughly text chars × 4 bytes; 32 steps are plenty here.
// Truly unlimited undo would require diffs instead of snapshots.
pub const CAPACITY: usize = 64;

// One editor snapshot: text + cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub chars: Vec<char>,
    pub cursor: usize,
}

// The kind of one edit step; decides whether it merges with the previous step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    // Consecutive single-character input (mergeable).
    Typing,
    // Everything else (deletion, paste, newline, bulk edits... not mergeable).
    Other,
}

#[derive(Debug, Clone, Default)]
pub struct UndoStack {
    // Undo stack: the last item is the state before the most recent edit.
    undo: Vec<Snapshot>,
    // Redo stack.
    redo: Vec<Snapshot>,
    // Kind of the previous edit, for merge decisions.
    last_kind: Option<EditKind>,
    // When the previous edit happened, for the time gate.
    last_at: Option<Instant>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    // Call **before** performing an edit: pushes the current state onto the undo stack.
    //
    // With `kind == Typing` right after another `Typing` within the window, no new
    // snapshot is pushed — consecutive character input merges into one undo unit.
    //
    // `now` is passed in instead of taking `Instant::now()` internally,
    // letting tests drive behavior deterministically.
    pub fn record(&mut self, before: Snapshot, kind: EditKind, now: Instant) {
        let coalesce = kind == EditKind::Typing
            && self.last_kind == Some(EditKind::Typing)
            && self
                .last_at
                .is_some_and(|t| now.duration_since(t) < COALESCE_WINDOW);

        self.last_kind = Some(kind);
        self.last_at = Some(now);

        if coalesce {
            // Merged into the previous step: no new snapshot, but the timestamp refreshed (two lines above)
            self.redo.clear();
            return;
        }

        self.undo.push(before);
        if self.undo.len() > CAPACITY {
            self.undo.remove(0); // drop the oldest step
        }
        // A new edit invalidates redo (standard semantics)
        self.redo.clear();
    }

    // Undo: `current` is the present state; returns the state to restore (pushed onto redo).
    //
    // `None` means nothing left to undo.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.undo.pop()?;
        self.redo.push(current);
        // Typing right after an undo must not merge with the pre-undo step
        self.break_coalescing();
        Some(prev)
    }

    // Redo: returns the state to restore (pushed back onto the undo stack).
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        self.break_coalescing();
        Some(next)
    }

    // Break merging: the next edit becomes its own undo unit.
    //
    // Should follow cursor movement — typing after moving is clearly not the same edit.
    pub fn break_coalescing(&mut self) {
        self.last_kind = None;
        self.last_at = None;
    }

    // Clear (e.g. resetting the editor after submission).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.break_coalescing();
    }
}
