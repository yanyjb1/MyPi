//! The block separator set: the one definition of where a word ends.
//!
//! Word delete, word motion and the completion trigger walk all stop here, so `src/main.rs`
//! and `a-b_c.tar.gz` are one block each.

use std::collections::HashSet;

/// The block boundary set, shared by the editor and the completion trigger.
#[derive(Debug, Clone)]
pub struct SeparatorTable {
    hard: HashSet<char>,
}

impl Default for SeparatorTable {
    fn default() -> Self {
        // Whitespace, CJK punctuation, and Latin punctuation; `./\-_` are connectors.
        let hard: &[char] = &[
            ' ', '\t', '\n', '\r', ',', ';', ':', '!', '?', '\'', '"', '(', ')', '[', ']', '{',
            '}', '<', '>', '|', '&', '%', '$', '#', '@', '=', '+', '*', '^', '`', '，', '。', '、',
            '；', '：', '？', '！', '“', '”', '‘', '’', '（', '）', '【', '】', '《', '》', '「',
            '」', '『', '』', '—', '…', '·', '～',
        ];
        Self {
            hard: hard.iter().copied().collect(),
        }
    }
}

impl SeparatorTable {
    /// Whether this character ends a block.
    pub fn is_hard(&self, c: char) -> bool {
        self.hard.contains(&c)
    }
}
