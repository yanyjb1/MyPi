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
//! The format matches pi exactly, so habits from pi transfer directly:
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

impl Marker {
}

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
                Some(content) => out.push_str(content),
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
    let id: u64 = chars[digits_from..i].iter().collect::<String>().parse().ok()?;

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
pub fn marker_spanning(chars: &[char], pos: usize, include_start: bool, include_end: bool) -> Option<Marker> {
    find_markers(chars).into_iter().find(|m| {
        let left_ok = if include_start { pos >= m.start } else { pos > m.start };
        let right_ok = if include_end { pos <= m.end } else { pos < m.end };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    // ---- parsing ----

    #[test]
    fn parses_all_three_shapes() {
        let c = chars("看 [paste #12 +30 lines] 和 [paste #7 1234 chars] 以及 [paste #3]");
        let ms = find_markers(&c);
        assert_eq!(ms.len(), 3);
        assert_eq!(ms[0].id, 12);
        assert_eq!(ms[0].start, 2);
        assert_eq!(&c[ms[0].start..ms[0].end].iter().collect::<String>(), "[paste #12 +30 lines]");
        assert_eq!(ms[1].id, 7);
        assert_eq!(ms[2].id, 3);
    }

    #[test]
    fn parses_bare_marker() {
        let c = chars("[paste #3]");
        let ms = find_markers(&c);
        assert_eq!(ms.len(), 1);
        assert_eq!((ms[0].start, ms[0].end), (0, 10));
    }

    #[test]
    fn ignores_lookalikes() {
        // Non-marker shapes must not be recognized
        for s in [
            "[paste #]",
            "[paste #abc]",
            "[paste #1",
            "[paste 1]",
            "paste #1]",
            "[paste #1 + lines]",
            "[paste #1 12 chars",
        ] {
            assert!(find_markers(&chars(s)).is_empty(), "不该匹配: {s}");
        }
    }

    #[test]
    fn handles_cjk_indices_as_chars_not_bytes() {
        // CJK text: 2 chars but 6 bytes; indexes must be char-based
        let c = chars("中文[paste #1 +30 lines]");
        let ms = find_markers(&c);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].start, 2, "下标应按字符计，不是字节");
        assert_eq!(ms[0].end, 2 + "[paste #1 +30 lines]".chars().count());
    }

    #[test]
    fn finds_adjacent_markers() {
        let c = chars("[paste #1 +30 lines][paste #2 +40 lines]");
        let ms = find_markers(&c);
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[0].end, ms[1].start, "紧挨着也要能都认出来");
    }

    // ---- threshold decision ----

    #[test]
    fn collapse_threshold_matches_pi() {
        assert!(!should_collapse("1\n2\n3"), "3 行不折叠");
        let ten: String = (0..10).map(|i| format!("L{i}\n")).collect(); // 11 行
        assert!(should_collapse(&ten), "11 行该折叠");
        assert!(!should_collapse(&"x".repeat(1000)), "1000 字符是边界内");
        assert!(should_collapse(&"x".repeat(1001)), "1001 字符该折叠");
    }

    #[test]
    fn marker_text_format_matches_pi() {
        assert_eq!(PasteStore::marker_text(1, 30, 900), "[paste #1 +30 lines]");
        // Under the line limit, show the character count
        assert_eq!(PasteStore::marker_text(2, 3, 5000), "[paste #2 5000 chars]");
    }

    // ---- store and restoration ----

    #[test]
    fn ids_are_never_reused() {
        let mut s = PasteStore::new();
        let a = s.add("aaa".into());
        let b = s.add("bbb".into());
        assert_ne!(a, b, "编号必须唯一，否则撤销后会串内容");
        assert_eq!(s.get(a), Some("aaa"));
        assert_eq!(s.get(b), Some("bbb"));
    }

    #[test]
    fn expand_restores_content() {
        let mut s = PasteStore::new();
        let id = s.add("line1\nline2\nline3".into());
        let marker = PasteStore::marker_text(id, 3, 17);
        let text = format!("看下这段 {marker} 有什么问题");
        assert_eq!(s.expand(&text), "看下这段 line1\nline2\nline3 有什么问题");
    }

    #[test]
    fn expand_keeps_unknown_marker_verbatim() {
        let s = PasteStore::new();
        let text = "a [paste #99 +30 lines] b";
        assert_eq!(s.expand(text), text, "找不到原文就原样保留，不能吞内容");
    }

    #[test]
    fn expand_handles_multiple_and_cjk() {
        let mut s = PasteStore::new();
        let a = s.add("AAA".into());
        let b = s.add("BBB".into());
        let text = format!(
            "中文{}中间{}尾巴",
            PasteStore::marker_text(a, 30, 3),
            PasteStore::marker_text(b, 40, 3)
        );
        assert_eq!(s.expand(&text), "中文AAA中间BBB尾巴");
    }

    #[test]
    fn expand_is_identity_without_markers() {
        let s = PasteStore::new();
        assert_eq!(s.expand("普通文本"), "普通文本");
        assert_eq!(s.expand(""), "");
    }

    #[test]
    fn clear_resets_ids() {
        let mut s = PasteStore::new();
        s.add("x".into());
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.add("y".into()), 1, "清空后编号应从头开始");
    }

    // ---- range expansion (the core of atomic deletion) ----

    #[test]
    fn expand_absorbs_partially_overlapped_marker() {
        let c = chars("ab[paste #1 +30 lines]cd");
        let (lo, hi) = expand_over_markers(&c, 0, 3); // 切在标记的第 1 个字符处
        assert_eq!((lo, hi), (0, 22), "部分覆盖必须扩张成完整删除");
        assert_eq!(&c[lo..hi].iter().collect::<String>(), "ab[paste #1 +30 lines]");
    }

    #[test]
    fn expand_from_inside_covers_whole_marker() {
        let c = chars("[paste #1 +30 lines]");
        let (lo, hi) = expand_over_markers(&c, 10, 11); // 正中间
        assert_eq!((lo, hi), (0, 20), "从中间删也要整块删掉");
    }

    #[test]
    fn expand_leaves_untouched_ranges_alone() {
        let c = chars("hello [paste #1 +30 lines] world");
        // Entirely left of the marker
        assert_eq!(expand_over_markers(&c, 0, 5), (0, 5));
        // Entirely right of the marker (marker spans 6..26, so 27 onward is clean)
        assert_eq!(expand_over_markers(&c, 27, 32), (27, 32));
    }

    #[test]
    fn expand_handles_adjacent_markers() {
        let c = chars("[paste #1 +30 lines][paste #2 +40 lines]");
        // The range straddles the boundary
        let (lo, hi) = expand_over_markers(&c, 19, 21);
        assert_eq!((lo, hi), (0, 40), "挨在一起的两个标记都要吃掉");
    }

    #[test]
    fn expand_is_noop_without_markers() {
        let c = chars("plain text");
        assert_eq!(expand_over_markers(&c, 1, 4), (1, 4));
    }

    // ---- position queries ----

    #[test]
    fn spanning_respects_endpoints() {
        let c = chars("x[paste #1 +30 lines]y");
        let m = find_markers(&c)[0];
        // Backspace case: cursor at the marker right edge (end) must hit
        assert!(marker_spanning(&c, m.end, true, true).is_some());
        // Forward-delete case: cursor at the marker left edge (start) must also hit
        assert!(marker_spanning(&c, m.start, true, true).is_some());
        // Strictly inside
        assert!(marker_spanning(&c, m.start + 5, true, true).is_some());
        // Outside both ends
        assert!(marker_spanning(&c, 0, true, true).is_none());
        assert!(marker_spanning(&c, m.end + 1, true, true).is_none());
        // Endpoints excluded: hugging the right edge does not count
        assert!(marker_spanning(&c, m.end, false, false).is_none());
        assert!(marker_spanning(&c, m.start, false, false).is_none());
    }
}
