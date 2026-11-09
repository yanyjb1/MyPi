//! The block boundary set: the one definition of where a word ends.
//!
//! Word motion and word delete stop at a hard character, or where the script changes; the
//! completion trigger looks at whole non-space runs instead.

use std::collections::HashSet;

/// The block boundary set, for word delete and word motion.
#[derive(Debug, Clone)]
pub struct SeparatorTable {
    hard: HashSet<char>,
}

/// One character's script family: a boundary sits between two families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// ASCII letters, digits and the Latin blocks.
    Latin,
    Han,
    Kana,
    Hangul,
    Greek,
    Cyrillic,
    /// Anything else that is not hard: connectors, emoji, stray symbols.
    Other,
}

impl Default for SeparatorTable {
    fn default() -> Self {
        // Whitespace, CJK punctuation, and Latin punctuation; `.\-_` are connectors.
        let hard: &[char] = &[
            ' ', '\t', '\n', '\r', ',', ';', ':', '!', '?', '\'', '"', '(', ')', '[', ']', '{',
            '}', '<', '>', '|', '&', '%', '$', '#', '@', '=', '+', '*', '^', '`', '/', '，', '。',
            '、', '；', '：', '？', '！', '“', '”', '‘', '’', '（', '）', '【', '】', '《', '》',
            '「', '」', '『', '』', '—', '…', '·', '～',
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

    /// Whether this character stays in a run of `class`: the same family, or a connector
    /// that joins whatever sits beside it.
    pub fn joins(&self, class: Class, c: char) -> bool {
        matches!(c, '.' | '_' | '-') || self.class(c) == class
    }

    /// One character's script family.
    pub fn class(&self, c: char) -> Class {
        match u32::from(c) {
            0x30..=0x39 | 0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F => Class::Latin,
            0x370..=0x3FF | 0x1F00..=0x1FFF => Class::Greek,
            0x400..=0x52F => Class::Cyrillic,
            0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => Class::Hangul,
            0x3040..=0x30FF | 0x31F0..=0x31FF => Class::Kana,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F => Class::Han,
            _ => Class::Other,
        }
    }
}
