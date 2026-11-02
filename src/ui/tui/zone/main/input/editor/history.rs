//! Input-history browsing — mirrors pi's `navigateHistory` / `exitHistoryBrowsing`
//! (`packages/tui/src/components/editor.ts:453`).
//!
//! Semantics (copied from pi so existing muscle memory transfers directly):
//!
//! - ↑ enters history when on the **first visual row** and the conditions hold;
//!   otherwise it is an ordinary cursor-up move.
//! - Entering history first saves the **content currently being written** as a draft.
//! - Walking ↓ past the newest entry exits history browsing and hands the draft back.
//! - Editing a text recalled from history makes it a new input and exits history mode.
//!
//! The last rule matters: without it, "recall an old command → tweak one char →
//! press ↑ again" would silently drop the edited text — highly counterintuitive.

/// Input history browser.
#[derive(Debug, Clone, Default)]
pub struct History {
    /// Texts submitted in the past, newest last.
    entries: Vec<String>,
    /// Index currently being browsed. `None` = not in history mode (writing new content).
    index: Option<usize>,
    /// Draft captured before entering history mode; restored on exit.
    draft: Option<String>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Twenty entries in memory, oldest dropped first; never stored.
    pub const CAP: usize = 20;

    /// Submit a new input. Exits history mode.
    pub fn push(&mut self, text: impl Into<String>) {
        let text = text.into();
        // Consecutive duplicates are not pushed twice (common readline behavior)
        if self.entries.last().map(|s| s.as_str()) != Some(text.as_str()) {
            self.entries.push(text);
            if self.entries.len() > Self::CAP {
                self.entries.remove(0);
            }
        }
        self.exit();
    }

    /// Whether history is currently being browsed.
    pub fn browsing(&self) -> bool {
        self.index.is_some()
    }

    /// Exit history mode (does not clear the history, just resets the cursor).
    pub fn exit(&mut self) {
        self.index = None;
        self.draft = None;
    }

    /// The user edited the input box content: leave history mode and detach from the draft.
    ///
    /// The draft is deliberately **not** restored here — the user has already modified
    /// the recalled text, so that is what they intend to write; stomping the draft back
    /// over it would be maddening.
    pub fn on_edit(&mut self) {
        self.index = None;
        self.draft = None;
    }

    /// ↑: previous history entry.
    ///
    /// `current` is the input box's current content (saved as the draft when first
    /// entering history). Returns `Some(text)` meaning the input box should be replaced
    /// with it; `None` means there is nothing older to recall.
    pub fn previous(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let new_index = match self.index {
            None => {
                self.draft = Some(current.to_string());
                self.entries.len() - 1
            }
            Some(0) => return None, // already at the oldest entry
            Some(i) => i - 1,
        };
        self.index = Some(new_index);
        Some(self.entries[new_index].clone())
    }

    /// ↓: next history entry. Moving past the newest one exits history mode and
    /// hands back the draft.
    ///
    /// Not named `next`: that would collide with `Iterator::next`, and readers
    /// (and future `for` loops) could mistake it for an iterator.
    pub fn next_entry(&mut self) -> Option<String> {
        let i = self.index?;
        if i + 1 < self.entries.len() {
            self.index = Some(i + 1);
            return Some(self.entries[i + 1].clone());
        }
        // Past the newest entry: exit history mode and restore the draft
        let draft = self.draft.take().unwrap_or_default();
        self.index = None;
        Some(draft)
    }
}
