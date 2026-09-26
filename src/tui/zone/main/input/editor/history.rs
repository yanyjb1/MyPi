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

    /// 内存环上限：最多保留 20 条，超出丢最旧。不落盘（用户决策）。
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

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn h_with(items: &[&str]) -> History {
        let mut h = History::new();
        for s in items {
            h.push(*s);
        }
        h
    }

    #[test]
    fn empty_history_yields_nothing() {
        let mut h = History::new();
        assert_eq!(h.previous("draft"), None);
        assert_eq!(h.next_entry(), None);
    }

    #[test]
    fn up_walks_backwards_through_entries() {
        let mut h = h_with(&["one", "two", "three"]);
        assert_eq!(h.previous(""), Some("three".into()));
        assert_eq!(h.previous(""), Some("two".into()));
        assert_eq!(h.previous(""), Some("one".into()));
        // ↑ at the oldest entry does nothing
        assert_eq!(h.previous(""), None);
        assert!(h.browsing());
    }

    #[test]
    fn down_walks_forward_and_restores_draft() {
        let mut h = h_with(&["one", "two"]);
        assert_eq!(h.previous("my draft"), Some("two".into()));
        assert_eq!(h.previous(""), Some("one".into()));
        assert_eq!(h.next_entry(), Some("two".into()));
        // Past the newest entry → back to the draft, and history mode exits
        assert_eq!(h.next_entry(), Some("my draft".into()));
        assert!(!h.browsing());
    }

    #[test]
    fn draft_is_captured_on_first_up_only() {
        let mut h = h_with(&["one", "two"]);
        // The first ↑ records the draft
        assert_eq!(h.previous("draft-A"), Some("two".into()));
        // The second ↑ must not overwrite the draft with "two"
        assert_eq!(h.previous("ignored"), Some("one".into()));
        assert_eq!(h.next_entry(), Some("two".into()));
        assert_eq!(h.next_entry(), Some("draft-A".into()));
    }

    #[test]
    fn editing_exits_browsing_without_restoring_draft() {
        let mut h = h_with(&["one", "two"]);
        assert_eq!(h.previous("draft"), Some("two".into()));
        h.on_edit(); // the user edited this text
        assert!(!h.browsing());
        // Pressing ↑ again starts over (current content becomes the new draft)
        assert_eq!(h.previous("edited"), Some("two".into()));
        assert_eq!(h.next_entry(), Some("edited".into()));
    }

    #[test]
    fn duplicate_consecutive_entries_are_deduped() {
        let mut h = History::new();
        h.push("same");
        h.push("same");
        h.push("other");
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn push_exits_browsing() {
        let mut h = h_with(&["one"]);
        h.previous("draft");
        assert!(h.browsing());
        h.push("two");
        assert!(!h.browsing());
        assert_eq!(h.previous(""), Some("two".into()));
    }

    #[test]
    fn next_without_browsing_is_none() {
        let mut h = h_with(&["one"]);
        assert_eq!(h.next_entry(), None);
    }

    #[test]
    fn two_consecutive_ups_walk_two_entries() {
        // Regression guard: the second ↑ must keep walking backwards (the browsing
        // state must not be cleared mid-walk). Reproduces the app layer's actual call
        // sequence: every call passes the editor's current content.
        let mut h = h_with(&["AAA", "BBB"]);
        let first = h.previous("").expect("第一次 ↑");
        assert_eq!(first, "BBB");
        assert!(h.browsing(), "第一次 ↑ 之后必须还在浏览态");
        // The app has put "BBB" into the editor, so the second call passes "BBB"
        let second = h.previous(&first).expect("第二次 ↑");
        assert_eq!(second, "AAA", "第二次 ↑ 应继续往旧翻");
    }
}
