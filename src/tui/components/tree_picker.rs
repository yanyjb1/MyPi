//! Conversation tree navigator modal (double-Esc, pi's /tree).
//!
//! Full-screen modal: while open it takes over ↑↓/Enter/Esc and the wheel;
//! the three base zones receive nothing. Confirming moves the store's leaf
//! pointer (append-only tree — nothing is deleted) and the app reprojects.

use crate::store::TreeNode;
use crate::entry::Entry;
use crate::tui::text::display_width;
use crate::tui::theme::Palette;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// One selectable row in the navigator (a stored entry, flattened).
pub struct TreeRow {
    pub seq: i64,
    pub parent_seq: Option<i64>,
    /// One-line human summary of the entry ("你: ...", "✓ edit", ...).
    pub label: String,
    /// Kind marker for styling ("user" | "assistant" | "tool" | "name" | ...).
    pub kind: &'static str,
    /// True when this row is the current leaf.
    pub is_leaf: bool,
}

pub struct TreePicker {
    /// All rows (whole tree, every branch).
    pub rows: Vec<TreeRow>,
    /// Highlighted index into `rows`.
    pub selected: usize,
    /// Current leaf seq (for the "you are here" marker).
    pub leaf: Option<i64>,
}

impl TreePicker {
    /// Build rows from the flat tree. Every stored entry is listed —
    /// including abandoned branches — in seq order; `is_leaf` marks the tip.
    pub fn from_tree(tree: &[TreeNode], leaf: Option<i64>) -> Self {
        let mut rows = Vec::with_capacity(tree.len());
        for n in tree {
            let e = Entry::from_payload(&n.kind, &n.payload);
            let (label, kind) = summarize(&n.kind, e.as_ref());
            rows.push(TreeRow {
                seq: n.seq,
                parent_seq: n.parent_seq,
                label,
                kind,
                is_leaf: Some(n.seq) == leaf,
            });
        }
        let selected = rows.iter().position(|r| r.is_leaf).unwrap_or(0);
        Self { rows, selected, leaf }
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as i32;
        let next = (self.selected as i32 + delta).rem_euclid(len);
        self.selected = next as usize;
    }

    /// The highlighted row's seq (what "confirm" navigates to).
    pub fn confirm(&self) -> Option<i64> {
        self.rows.get(self.selected).map(|r| r.seq)
    }
}

// One-line summary per entry kind. Vague by design: "大概干了啥" is enough.
fn summarize(kind: &str, e: Option<&Entry>) -> (String, &'static str) {
    match (kind, e) {
        (_, Some(Entry::User { content })) => {
            let first = content.lines().next().unwrap_or("");
            (truncate(first, 60), "user")
        }
        (_, Some(Entry::Assistant { content, .. })) => {
            let first = content.lines().next().unwrap_or("");
            (truncate(first, 60), "assistant")
        }
        (_, Some(Entry::ToolRequest { name, .. })) => (format!("⚙ {name}"), "tool"),
        (_, Some(Entry::ToolResult { name, ok, .. })) => {
            (format!("{} {name}", if *ok { "✓" } else { "✗" }), "tool")
        }
        (_, Some(Entry::Name { name })) => (format!("🏷 {name}"), "name"),
        _ => (kind.to_string(), "meta"),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        s.to_string()
    } else {
        let mut out = String::new();
        let mut w = 0;
        for c in s.chars() {
            let cw = crate::tui::text::char_width(c);
            if w + cw > max - 1 {
                break;
            }
            out.push(c);
            w += cw;
        }
        out.push('…');
        out
    }
}

// Render the whole modal. Returns the lines; the caller draws them over
// the full screen area.
pub fn render(picker: &TreePicker, _w: u16, h: u16, p: &Palette) -> Vec<Line<'static>> {
    let _ = p;
    let _ = _w; // palette reserved for per-kind styling below
    let mut out = Vec::new();
    out.push(Line::from(vec![
        Span::styled(" 会话树 ", Style::new().fg(Color::Black).bg(Color::Rgb(0, 200, 120)).add_modifier(Modifier::BOLD)),
        Span::styled(" ↑↓ 移动 · Enter 回到此节点 · Esc 退出 ", Style::new().fg(Color::DarkGray)),
    ]));
    out.push(Line::from(""));
    let rows = h.saturating_sub(3) as usize;
    let start = picker.selected.saturating_sub(rows.saturating_sub(1)).min(picker.rows.len().saturating_sub(1).min(picker.selected));
    for (i, r) in picker.rows.iter().enumerate().skip(start).take(rows) {
        let marker = if i == picker.selected { "> " } else { "  " };
        let branch_note = match r.parent_seq {
            Some(pseq) => {
                // A fork point: the previous row is not our parent.
                match i.checked_sub(1).and_then(|j| picker.rows.get(j)) {
                    Some(prev) if prev.seq != pseq => format!("⑂<-#{pseq} "),
                    _ => String::new(),
                }
            }
            None => "root ".to_string(),
        };
        let leaf_note = if r.is_leaf { " ●" } else { "" };
        let style = if i == picker.selected {
            Style::new().fg(Color::Black).bg(Color::Rgb(0, 200, 120))
        } else if r.is_leaf {
            Style::new().fg(Color::Green)
        } else if r.kind == "user" {
            Style::new().fg(Color::White)
        } else {
            Style::new().fg(Color::DarkGray)
        };
        out.push(Line::from(Span::styled(
            format!("{marker}#{:<4}{}{}{}", r.seq, branch_note, r.label, leaf_note),
            style,
        )));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(seq: i64, parent: Option<i64>, kind: &str, payload: &str) -> TreeNode {
        TreeNode { seq, parent_seq: parent, kind: kind.into(), payload: payload.into() }
    }

    #[test]
    fn rows_cover_all_branches_and_mark_leaf() {
        let tree = [
            node(1, None, "user", r#"{"content":"a"}"#),
            node(2, Some(1), "assistant", r#"{"content":"old"}"#),
            node(3, Some(1), "assistant", r#"{"content":"new"}"#),
        ];
        let picker = TreePicker::from_tree(&tree, Some(3));
        assert_eq!(picker.rows.len(), 3);
        assert!(picker.rows[2].is_leaf);
        assert!(!picker.rows[1].is_leaf);
        assert_eq!(picker.selected, 2); // starts at the leaf
        assert_eq!(picker.confirm(), Some(3));
    }

    #[test]
    fn selection_wraps() {
        let tree = [
            node(1, None, "user", r#"{"content":"a"}"#),
            node(2, Some(1), "user", r#"{"content":"b"}"#),
        ];
        let mut picker = TreePicker::from_tree(&tree, Some(2));
        picker.move_selection(1);
        assert_eq!(picker.selected, 0); // wraps to top
        picker.move_selection(-1);
        assert_eq!(picker.selected, 1);
    }

    #[test]
    fn summarize_maps_kinds() {
        let (_, k) = summarize("user", Some(&Entry::User { content: "你好".into() }));
        assert_eq!(k, "user");
        let (_, k) = summarize("tool_result", Some(&Entry::ToolResult {
            call_id: "c".into(), name: "edit".into(), ok: true, result: String::new(),
        }));
        assert_eq!(k, "tool");
    }
}
