//! Fold markers for large pastes — mirrors pi's paste marker
//! (`packages/tui/src/components/editor.ts`, around `PASTE_MARKER_REGEX`).
//!
//! Problem: pasting 30 lines of code fills the whole input area (capped at a
//! quarter of the terminal height), looks messy, and deleting takes 30 backspaces.
//!
//! Solution: a paste above the threshold **folds into one marker** `[paste #1 +30 lines]`,
//! with the original stored in `PasteStore`. The marker is **atomic** in the editor:
//! one backspace removes it all, one arrow key crosses it, and submission restores the full text.
//!
//! On submit the marker expands back to the original, **fenced**:
//!
//! ````text
//! ```pasted content
//! <the original text>
//! ```
//! ````
//!
//! The fence is on its own lines and carries the `pasted content` tag, so the
//! model can tell pasted text from what the user typed, and pasted markdown
//! cannot swallow the sentence around it.
//!
//! The marker format matches pi exactly, so habits from pi transfer directly:
//! ```text
//! [paste #1 +123 lines]    more than 10 lines
//! [paste #1 1234 chars]    few lines but many characters
//! ```

use std::collections::HashMap;

// Fold above this line count.
pub const MAX_LINES: usize = 10;
// Also fold when the character count exceeds this (base64 blobs in one line, etc.).
pub const MAX_CHARS: usize = 1000;

// The marker's fixed prefix.
const PREFIX: &str = "[paste #";

// Position of a folded marker within the character sequence.
//
// `start`/`end` are **char indexes** (not bytes); `end` is exclusive,
// matching the editor's `Vec<char>` model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub id: u64,
    pub start: usize,
    pub end: usize,
}

impl Marker {}

// The store of folded originals.
//
// Deliberately **append-only**: numbers are never reused, content never deleted.
//
// Why this is safe: after undo/redo an old marker text reappears and its content
// is still in the store, so it works automatically — no need to snapshot the store,
// and a marker can never dangle after an undo.
// The cost is a few orphan strings per session, all cleared on submit.
#[derive(Debug, Clone, Default)]
pub struct PasteStore {
    entries: HashMap<u64, String>,
    // Next number. Monotonic — reusing numbers would break the invariant above.
    next_id: u64,
}

impl PasteStore {
    pub fn new() -> Self {
        Self::default()
    }

    // Store an original and return its number.
    pub fn add(&mut self, text: String) -> u64 {
        self.next_id += 1;
        self.entries.insert(self.next_id, text);
        self.next_id
    }

    pub fn get(&self, id: u64) -> Option<&str> {
        self.entries.get(&id).map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // Clear everything (called after submission).
    //
    // The counter resets too: the editor is cleared at that point, so no marker can reference old numbers.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.next_id = 0;
    }

    // Build the marker text. `line_count` is the original line count, `char_count` the character count.
    //
    // Decision order matches pi: show lines when over the limit, otherwise characters.
    pub fn marker_text(id: u64, line_count: usize, char_count: usize) -> String {
        if line_count > MAX_LINES {
            format!("{PREFIX}{id} +{line_count} lines]")
        } else {
            format!("{PREFIX}{id} {char_count} chars]")
        }
    }

    // Restore every marker in the text to its original. A marker whose original is
    // missing is **kept verbatim** — visible text beats silently dropping content.
    pub fn expand(&self, text: &str) -> String {
        if !text.contains(PREFIX) {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let markers = find_markers(&chars);
        if markers.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut cursor = 0usize;
        for m in &markers {
            // Copy the original text before the marker verbatim
            out.extend(&chars[cursor..m.start]);
            match self.get(m.id) {
                Some(content) => {
                    // Fence it, on its own lines. The marker can sit
                    // mid-sentence ("看下这段 [paste #1] 有什么问题"), and raw
                    // pasted text often starts with `#` or `-` — spliced
                    // inline, it would eat the surrounding sentence. The
                    // fence also tells the model this text arrived by paste.
                    out.push_str("\n```pasted content\n");
                    out.push_str(content);
                    if !content.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n");
                }
                None => out.extend(&chars[m.start..m.end]), // missing original: keep verbatim
            }
            cursor = m.end;
        }
        out.extend(&chars[cursor..]);
        out
    }
}

// Whether this text should be folded.
pub fn should_collapse(text: &str) -> bool {
    let lines = text.split('\n').count();
    let chars = text.chars().count();
    lines > MAX_LINES || chars > MAX_CHARS
}

// Scan out every valid marker in the text (ascending by position).
//
// Hand-written scan instead of a regex: no regex dependency for this,
// and regexes work on bytes while we need char indexes — conversion would be messier.
pub fn find_markers(chars: &[char]) -> Vec<Marker> {
    let prefix: Vec<char> = PREFIX.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + prefix.len() <= chars.len() {
        if chars[i..i + prefix.len()] == prefix[..]
            && let Some((id, end)) = parse_marker_at(chars, i)
        {
            out.push(Marker { id, start: i, end });
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

// Try to parse one marker at `start`; returns `(id, end index)`.
//
// Accepted shape (equivalent to pi's regex `\[paste #(\d+)( (\+\d+ lines|\d+ chars))?\]`):
// ```text
// [paste #12]
// [paste #12 +30 lines]
// [paste #12 1234 chars]
// ```
fn parse_marker_at(chars: &[char], start: usize) -> Option<(u64, usize)> {
    let mut i = start + PREFIX.chars().count();

    // Number: at least one digit
    let digits_from = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_from {
        return None;
    }
    let id: u64 = chars[digits_from..i]
        .iter()
        .collect::<String>()
        .parse()
        .ok()?;

    // Optional quantity segment: ` +N lines` or ` N chars`
    // Fall back on parse failure; `]` may directly follow the number
    if i < chars.len() && chars[i] == ' ' {
        let save = i;
        i += 1;
        if i < chars.len() && chars[i] == '+' {
            i += 1;
        }
        let num_from = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == num_from || i >= chars.len() || chars[i] != ' ' {
            i = save; // not a valid quantity segment
        } else {
            i += 1; // skip the space after the number
            // Unit word: lines or chars, fully matched and directly followed by `]`
            // The unit must be lines or chars, immediately followed by `]`.
            // Both units have the same length; only which-one matters, no differing advances.
            let rest: String = chars[i..].iter().take(5).collect();
            if rest == "lines" || rest == "chars" {
                i += 5;
            } else {
                i = save;
            }
        }
    }

    // The closing `]`
    if i < chars.len() && chars[i] == ']' {
        Some((id, i + 1))
    } else {
        None
    }
}

// Find the marker straddling `pos` (endpoints optionally).
//
// `include_end` decides whether `pos == marker.end` counts:
// - Backspace (cursor at the marker's **right** edge) needs it -> `include_end = true`
// - Forward delete (cursor at the **left** edge) uses the `include_start` side.
pub fn marker_spanning(
    chars: &[char],
    pos: usize,
    include_start: bool,
    include_end: bool,
) -> Option<Marker> {
    find_markers(chars).into_iter().find(|m| {
        let left_ok = if include_start {
            pos >= m.start
        } else {
            pos > m.start
        };
        let right_ok = if include_end {
            pos <= m.end
        } else {
            pos < m.end
        };
        left_ok && right_ok
    })
}

// Expand `[start, end)` outward until it fully covers every marker it
// partially touches.
//
// This is the core of atomic deletion: however the range cuts a marker,
// the result removes it whole. Iterating to a fixpoint is required
// because the expanded range may reach adjacent markers (they can sit
// right next to each other).
pub fn expand_over_markers(chars: &[char], start: usize, end: usize) -> (usize, usize) {
    let markers = find_markers(chars);
    if markers.is_empty() {
        return (start, end);
    }
    let mut lo = start;
    let mut hi = end;
    loop {
        let mut changed = false;
        for m in &markers {
            // Actual overlap (not mere contact) -> absorb the whole marker
            if lo < m.end && hi > m.start {
                if m.start < lo {
                    lo = m.start;
                    changed = true;
                }
                if m.end > hi {
                    hi = m.end;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    (lo, hi)
}
