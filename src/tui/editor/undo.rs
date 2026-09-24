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

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(s: &str, cursor: usize) -> Snapshot {
        Snapshot {
            chars: s.chars().collect(),
            cursor,
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn undo_without_history_returns_none() {
        let mut u = UndoStack::new();
        assert!(!u.can_undo());
        assert!(u.undo(snap("a", 1)).is_none());
    }

    #[test]
    fn typing_coalesces_into_one_undo_unit() {
        let mut u = UndoStack::new();
        let t = t0();
        // Simulate typing "abc": record before each edit
        u.record(snap("", 0), EditKind::Typing, t);
        u.record(
            snap("a", 1),
            EditKind::Typing,
            t + Duration::from_millis(50),
        );
        u.record(
            snap("ab", 2),
            EditKind::Typing,
            t + Duration::from_millis(100),
        );

        // Exactly 1 snapshot -> one undo returns to empty
        assert_eq!(u.undo.len(), 1);
        assert_eq!(u.undo(snap("abc", 3)), Some(snap("", 0)));
        assert!(!u.can_undo());
    }

    #[test]
    fn time_gap_breaks_coalescing() {
        let mut u = UndoStack::new();
        let t = t0();
        u.record(snap("", 0), EditKind::Typing, t);
        // A gap beyond the window: no merge
        u.record(
            snap("a", 1),
            EditKind::Typing,
            t + COALESCE_WINDOW + Duration::from_millis(1),
        );
        assert_eq!(u.undo.len(), 2);
        assert_eq!(u.undo(snap("ab", 2)), Some(snap("a", 1)));
        assert_eq!(u.undo(snap("a", 1)), Some(snap("", 0)));
    }

    #[test]
    fn other_edit_kind_never_coalesces() {
        let mut u = UndoStack::new();
        let t = t0();
        u.record(snap("abc", 3), EditKind::Typing, t);
        // A deletion: even immediately after, it must be its own step
        u.record(snap("ab", 2), EditKind::Other, t);
        assert_eq!(u.undo.len(), 2);
    }

    #[test]
    fn break_coalescing_starts_new_unit() {
        let mut u = UndoStack::new();
        let t = t0();
        u.record(snap("", 0), EditKind::Typing, t);
        u.break_coalescing(); // simulate a cursor move
        u.record(
            snap("a", 1),
            EditKind::Typing,
            t + Duration::from_millis(10),
        );
        assert_eq!(
            u.undo.len(),
            2,
            "typing after a cursor move must be its own step"
        );
    }

    #[test]
    fn redo_restores_undone_state() {
        let mut u = UndoStack::new();
        let t = t0();
        u.record(snap("", 0), EditKind::Typing, t);

        let after_undo = u.undo(snap("abc", 3)).unwrap();
        assert_eq!(after_undo, snap("", 0));
        assert!(u.can_redo());

        let after_redo = u.redo(snap("", 0)).unwrap();
        assert_eq!(after_redo, snap("abc", 3));
        assert!(!u.can_redo());
        assert!(u.can_undo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut u = UndoStack::new();
        let t = t0();
        u.record(snap("", 0), EditKind::Typing, t);
        u.undo(snap("a", 1));
        assert!(u.can_redo());
        // New input after undo -> the redo chain breaks
        u.record(snap("", 0), EditKind::Other, t);
        assert!(!u.can_redo());
    }

    #[test]
    fn undo_breaks_coalescing_so_next_type_is_new_unit() {
        let mut u = UndoStack::new();
        let t = t0();
        u.record(snap("", 0), EditKind::Typing, t);
        let _ = u.undo(snap("a", 1));
        // Typing right after undo: must be its own step, or it would swallow the restored state
        u.record(snap("", 0), EditKind::Typing, t + Duration::from_millis(10));
        assert_eq!(u.undo.len(), 1);
        let _ = u.undo(snap("x", 1));
        assert!(!u.can_undo());
    }

    #[test]
    fn capacity_drops_oldest() {
        let mut u = UndoStack::new();
        let t = t0();
        for i in 0..(CAPACITY + 10) {
            u.record(snap(&"x".repeat(i), 0), EditKind::Other, t);
        }
        assert_eq!(
            u.undo.len(),
            CAPACITY,
            "stack depth must not exceed the cap"
        );
    }

    #[test]
    fn clear_wipes_both_stacks() {
        let mut u = UndoStack::new();
        let t = t0();
        u.record(snap("", 0), EditKind::Other, t);
        u.undo(snap("a", 1));
        u.clear();
        assert!(!u.can_undo() && !u.can_redo());
    }
}
