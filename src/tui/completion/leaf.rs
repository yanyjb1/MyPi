//! Leaf-state resolution — the single source of "is this completion done".
//!
//! A popup in a **leaf state** must close: there is nothing left to extend,
//! and keeping it open would swallow Tab/Enter in a confirm-loop (the same
//! completion reapplied forever; Enter never submits again). Every rule that
//! decides "done" lives in this one pure function — new `ArgKind`s or
//! completion sources add a branch **here**, not another ad-hoc check in the
//! caller.

use super::engine::Completion;

/// Why the popup should close even though candidates exist (or None).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafReason {
    /// The single candidate's replacement equals the word being completed
    /// ("zstd.h" -> "zstd.h", "src/" -> "src/"): nothing to add.
    ExactWord,
    /// The word is already a full command name ("/q", "/model"): Enter must
    /// submit in one step, not re-confirm the completion.
    ExactCommand,
    /// The single argument candidate reproduces the whole line verbatim
    /// ("/switch global:..." applied → the id list offers the same line):
    /// nothing left to extend.
    ExactLine,
}

/// Decide whether the completion has reached a leaf state.
///
/// - `items`: the candidates computed for the current word
/// - `word`: the word being completed (what `candidate()` returned)
/// - `at_line_start`: the word starts the line (command-completion territory)
/// - `text`: the **whole input line** (needed for the ExactLine argument rule)
///
/// `COMMANDS` is consulted internally; the caller passes it implicitly
/// through the type system (no runtime duplication of the table).
pub fn leaf_state(
    items: &[Completion],
    word: &str,
    at_line_start: bool,
    text: &str,
) -> Option<LeafReason> {
    if items.is_empty() {
        return None; // nothing to show — the caller closes for "no candidates", not leaf
    }
    // 1. Argument leaf first: the single candidate reproduces the whole
    //    line verbatim ("/switch global:x" applied → id list offers the
    //    same line). When the word happens to be the whole line, this is
    //    the more precise reason than ExactWord.
    if items.len() == 1 && items[0].insert == text {
        return Some(LeafReason::ExactLine);
    }
    // 2. File/dir completed to a leaf: single candidate replaces the word
    //    with itself.
    if items.len() == 1 && items[0].insert == word {
        return Some(LeafReason::ExactWord);
    }
    // 3. Exact command-name hit: no longer listed alongside longer commands,
    //    so Enter submits in one step instead of confirming first.
    if at_line_start && is_exact_command(word) {
        return Some(LeafReason::ExactCommand);
    }
    None
}

/// Exact command hit (shared by `leaf_state` and callers that need the
/// boolean alone). Single scan of the static table.
pub fn is_exact_command(word: &str) -> bool {
    COMMANDS.iter().any(|c| c.name == word)
}

// Convenience re-export so callers need only this module.
use super::engine::COMMANDS;

#[cfg(test)]
mod tests {
    use super::*;

    fn c(insert: &str) -> Completion {
        Completion { name: String::new(), detail: String::new(), is_dir: false, insert: insert.into() }
    }

    #[test]
    fn exact_word_leaf() {
        // "zstd.h" -> "zstd.h": single candidate equals the word.
        let items = vec![c("zstd.h")];
        assert_eq!(leaf_state(&items, "zstd.h", false, "看下 zstd.h"), Some(LeafReason::ExactWord));
        // Two candidates: not a leaf.
        let items = vec![c("zstd.h"), c("zstd.hpp")];
        assert_eq!(leaf_state(&items, "zstd.h", false, "x"), None);
    }

    #[test]
    fn exact_command_leaf() {
        // "/q" is a full command name: leaf (only at line start).
        let items = vec![c("/q ")];
        assert_eq!(leaf_state(&items, "/q", true, "/q"), Some(LeafReason::ExactCommand));
        // Mid-line: command rules do not apply.
        assert_eq!(leaf_state(&items, "/q", false, "x /q"), None);
        // "/qu" is a prefix, not an exact hit.
        assert_eq!(leaf_state(&items, "/qu", true, "/qu"), None);
    }

    #[test]
    fn exact_line_leaf_catches_model_argument() {
        // The /switch regression: after applying "/switch global:m", the id
        // list re-offers the same line. Must be a leaf, not a re-open.
        let line = "/switch global:gpt-5.6-luna";
        let items = vec![c(line)];
        assert_eq!(leaf_state(&items, line, true, line), Some(LeafReason::ExactLine));
        // A partial id still lists real candidates: not a leaf.
        let items = vec![c("/switch global:a"), c("/switch global:b")];
        assert_eq!(leaf_state(&items, "/switch global:", true, "/switch global:"), None);
    }

    #[test]
    fn empty_items_are_not_leaf() {
        // No candidates is "close because empty", a different reason.
        assert_eq!(leaf_state(&[], "/x", true, "/x"), None);
    }
}
