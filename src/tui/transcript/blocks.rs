//! Block-based transcript rendering — the bounded-work path behind the chat view.
//!
//! Why this exists: the naive path re-rendered the *whole* transcript and
//! re-wrapped every row on every frame, then sliced a viewport out of the
//! result. Work per frame grew with total conversation length — fine for
//! ten rounds, a slideshow for two hundred. This module splits the work:
//!
//! 1. **Blocks.** A block is one transcript node as the user sees it: a
//!    user card, an assistant chunk (reasoning + reply), a glued tool
//!    exchange, a system notice. `blocks()` groups the entries once —
//!    request+result pairs become a single block, so the grouping and the
//!    rendering can never disagree about what "one node" is.
//! 2. **Measured rows.** Each block renders to rows that are **already
//!    hard-wrapped to the terminal width**, and reports its own height.
//!    Heights are bookkept in a prefix-sum (`Roster`), so the viewport
//!    math (`view.rs`) never needs the rows themselves — total scrollable
//!    height is O(blocks), not O(rows).
//! 3. **A bounded cache.** `BlockCache` materializes blocks on demand and
//!    evicts least-recently-used ones past a row budget, newest-first
//!    biased (the bottom of the conversation is where the user lives).
//!    Scrolling into ancient history renders just those blocks; memory
//!    stays flat no matter how long the session runs.
//!
//! Future-proofing for regenerate/tree ops: a block is identified by its
//! transcript index, so "everything after entry N changed" is exactly
//! "truncate the roster at N" — no special casing anywhere.

use ratatui::text::Line;

use super::components;
use crate::entry::Entry;
use crate::tui::theme::Palette;

/// One rendered node: wrapped rows + exact height.
pub(crate) struct Block {
    /// Hard-wrapped rows, exactly `width` cells (or narrower for plain rows).
    pub(crate) rows: Vec<Line<'static>>,
    /// Display height after wrapping — `rows.len()`, kept separate so the
    /// roster can be rebuilt from heights alone on a width change.
    pub(crate) height: usize,
}

// Grouping (`blocks`/`Range`) lives in `crate::grouping` — it is pure
// domain logic shared with the compactor, not a renderer concern.
pub use crate::grouping::{Range, blocks};

/// Render one block to width-wrapped rows. The single funnel every node
/// kind passes through — `single_node` renders the group, this wraps it.
pub(crate) fn render_block(
    entries: &[Entry],
    r: Range,
    p: &Palette,
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
) -> Block {
    let group = &entries[r.start..r.end];
    let mut lines = components::chat::single_node(group, p, show_reasoning, tools_expanded, width);
    // Wrap now, once per cache fill — not once per frame.
    let rows = wrap_rows(lines.drain(..), width);
    Block {
        height: rows.len(),
        rows,
    }
}

/// Hard-wrap rendered rows to `width` cells (moved out of `view.rs` so the
/// cache stores post-wrap rows and heights are exact, not estimated).
fn wrap_rows(lines: impl Iterator<Item = Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let w = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        let total: usize = line
            .spans
            .iter()
            .map(|s| crate::tui::text::display_width(&s.content))
            .sum();
        if total <= w {
            out.push(line);
            continue;
        }
        let mut cur: Vec<ratatui::text::Span<'static>> = Vec::new();
        let mut cur_w = 0usize;
        for sp in line.spans {
            let mut buf = String::new();
            for ch in sp.content.chars() {
                let cw = crate::tui::text::display_width(&ch.to_string());
                if cur_w + cw > w {
                    if !buf.is_empty() {
                        cur.push(ratatui::text::Span::styled(
                            std::mem::take(&mut buf),
                            sp.style,
                        ));
                    }
                    out.push(Line::from(std::mem::take(&mut cur)));
                    cur_w = 0;
                }
                buf.push(ch);
                cur_w += cw;
            }
            if !buf.is_empty() {
                cur.push(ratatui::text::Span::styled(buf, sp.style));
            }
        }
        if !cur.is_empty() {
            out.push(Line::from(cur));
        }
    }
    out
}

/// The separator between two blocks: one blank row, globally uniform.
pub(super) fn block_gap() -> Line<'static> {
    Line::from("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Palette {
        Palette::default()
    }

    fn user(t: &str) -> Entry {
        Entry::User { content: t.into() }
    }

    fn req(id: &str) -> Entry {
        Entry::ToolRequest {
            call_id: id.into(),
            name: "bash".into(),
            args: r#"{"command":"ls"}"#.into(),
            intent: String::new(),
            text: String::new(),
            first: true,
        }
    }

    fn res(id: &str) -> Entry {
        Entry::ToolResult {
            call_id: id.into(),
            name: "bash".into(),
            ok: true,
            result: "out".into(),
        }
    }

    #[test]
    fn request_result_pairs_glue_into_one_block() {
        let es = vec![user("hi"), req("c1"), res("c1"), user("again")];
        let bs = blocks(&es);
        assert_eq!(bs.len(), 3, "请求+结果必须是一个节点: {bs:?}");
        assert_eq!(bs[1], Range { start: 1, end: 3 });
    }

    #[test]
    fn unpaired_result_is_its_own_block() {
        let es = vec![req("c1"), user("x"), res("c1")];
        let bs = blocks(&es);
        assert_eq!(bs.len(), 3, "隔开的成对项不能跨块粘连: {bs:?}");
    }

    #[test]
    fn block_rows_fit_the_width_exactly() {
        let es = vec![user(
            "一段很长很长的中文消息用来测试折行 behaviour with mixed 文本 abcdefgh",
        )];
        let b = render_block(&es, Range { start: 0, end: 1 }, &p(), true, false, 20);
        assert!(b.height >= 3, "用户卡至少三行: {}", b.height);
        for l in &b.rows {
            let w: usize = l
                .spans
                .iter()
                .map(|s| crate::tui::text::display_width(&s.content))
                .sum();
            assert!(w <= 20, "行宽 {w} 超出 20");
        }
    }

    #[test]
    fn long_user_text_wraps_and_keeps_every_char() {
        let text = "一二三四五六七八九十".repeat(5);
        let es = vec![user(&text)];
        let b = render_block(&es, Range { start: 0, end: 1 }, &p(), true, false, 10);
        let joined: String = b
            .rows
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert_eq!(joined.replace(['▌', ' '], ""), text, "折行不得丢字");
    }
}
