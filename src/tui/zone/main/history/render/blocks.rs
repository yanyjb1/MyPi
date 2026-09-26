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

use super::chat;
use super::highlight;
use crate::server::entry::Entry;
use super::theme::HistoryTheme;

/// 一段**推迟上色**的行（渲染时按纯文本出图的那部分）。
///
/// `before`/`count` 是它在渲染器**折行之前**的行序列里的位置。上色只改样式、
/// 不改文本，折行逐行独立——所以把它按段单独折行，行数与整段一次折行**逐行
/// 相同**，块内行区间照用。
pub struct Deferred {
    pub(crate) before: usize,
    pub(crate) count: usize,
    /// 这一段的源码（上色时要原样再喂给 syntect）。
    pub(crate) code: String,
    /// 语言提示（`None` = 围栏没写语言）。
    pub(crate) lang: Option<String>,
}

/// 一个还没上色的段：块内**行区间** + 它的源码与语言。
struct Pending {
    rows: std::ops::Range<usize>,
    code: String,
    lang: Option<String>,
}

/// One rendered node: wrapped rows + exact height.
pub(crate) struct Block {
    /// Hard-wrapped rows, exactly `width` cells (or narrower for plain rows).
    pub(crate) rows: Vec<Line<'static>>,
    /// Display height after wrapping — `rows.len()`, kept separate so the
    /// roster can be rebuilt from heights alone on a width change.
    pub(crate) height: usize,
    /// 排版宽度：上色要按同一宽度重排，行数才不会变。
    width: usize,
    /// 还没上色的段。空 = 没有待办（也就没有后续工作）。
    pending: Vec<Pending>,
}

impl Block {
    /// 给落在块内行区间 `[lo, hi)` 的待上色段上色；返回这次色了几段。
    ///
    /// **从下往上**：段按行号倒序处理——读者盯的是底部，底部先出色。
    /// 上色只改样式（行数与逐行文本不变），所以行区间与高度都不用动，缓存的
    /// 几何一律不作废。上完的段就从待办里摘掉：下一帧直接读缓存的那几行。
    pub(crate) fn color_rows(&mut self, lo: usize, hi: usize, t: &HistoryTheme) -> usize {
        if self.pending.is_empty() || hi <= lo {
            return 0;
        }
        let mut colored = 0usize;
        let mut still: Vec<Pending> = Vec::new();
        for p in std::mem::take(&mut self.pending).into_iter().rev() {
            if p.rows.start >= hi || p.rows.end <= lo {
                still.push(p);
                continue;
            }
            let rows = wrap_rows(
                highlight::highlight(&p.code, p.lang.as_deref(), t).into_iter(),
                self.width,
            );
            // 行数必须一致——不一致说明"上色不改行数"这条前提被破坏了，
            // 那就宁可不换色（换了会让几何错位）。
            debug_assert_eq!(
                rows.len(),
                p.rows.len(),
                "上色改变了行数：{} vs {}",
                rows.len(),
                p.rows.len()
            );
            if rows.len() == p.rows.len() {
                self.rows[p.rows.clone()].clone_from_slice(&rows);
                colored += 1;
            }
        }
        still.reverse();
        self.pending = still;
        colored
    }

    /// 还有几段没上色（诊断与测试用）。
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

// Grouping (`blocks`/`Range`) lives in `crate::grouping` — it is pure
// domain logic shared with the compactor, not a renderer concern.
pub use crate::grouping::{Range, blocks};

/// Render one entry *group* to width-wrapped rows: the cache-facing wrapper.
/// `chat::single_node` maps the group to a block kind and calls the funnel
/// ([`super::render_block`]); this only wraps the result.
pub(crate) fn render_group(
    entries: &[Entry],
    r: Range,
    t: &HistoryTheme,
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
    defer: bool,
) -> Block {
    let group = &entries[r.start..r.end];
    let (mut lines, deferred) =
        chat::single_node(group, t, show_reasoning, tools_expanded, width, defer);
    // Wrap now, once per cache fill — not once per frame.
    //
    // 折行是**逐行独立**的（`wrap_rows` 每行各自切），所以"按段分开折行"与
    // "整段一次折行"逐行完全相同；分段唯一的目的是把每个待上色段落在哪几行
    // 记下来（上色时只换那几行）。
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    let mut it = lines.drain(..);
    let mut consumed = 0usize;
    for d in deferred {
        let gap = d.before.saturating_sub(consumed);
        rows.extend(wrap_rows(it.by_ref().take(gap), width));
        consumed += gap;
        let lo = rows.len();
        rows.extend(wrap_rows(it.by_ref().take(d.count), width));
        consumed += d.count;
        pending.push(Pending {
            rows: lo..rows.len(),
            code: d.code,
            lang: d.lang,
        });
    }
    rows.extend(wrap_rows(it.by_ref(), width));
    Block {
        height: rows.len(),
        rows,
        width,
        pending,
    }
}

/// Hard-wrap rendered rows to `width` cells (moved out of `view.rs` so the
/// cache stores post-wrap rows and heights are exact, not estimated).
///
/// `pub(super)` = the render module and its children: the streaming tail
/// (`chat::live_tail`) folds its rows exactly like the cache does, so the
/// live half-sentence and the finalized entry it becomes wrap identically.
pub(super) fn wrap_rows(
    lines: impl Iterator<Item = Line<'static>>,
    width: usize,
) -> Vec<Line<'static>> {
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
pub(crate) fn block_gap() -> Line<'static> {
    Line::from("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> HistoryTheme {
        HistoryTheme::resolve()
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
            details: None,
            duration_ms: 0,
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

    /// 每一格的（文本, 样式）。
    fn cells(b: &Block) -> Vec<(String, ratatui::style::Style)> {
        b.rows
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| (s.content.to_string(), s.style)))
            .collect()
    }

    fn rows_text(b: &Block) -> String {
        b.rows
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 推迟上色 + 事后补色 ≡ 一开始就上色：高度、逐行文本、每一格的样式
    /// 全都一致。
    ///
    /// 块缓存的几何（锚点、视口、窗口走查）全靠这条：上色只换样式，不改行数
    /// 与文本，所以补色可以在任何一帧插进来，位置一个都不动。
    #[test]
    fn deferring_then_coloring_equals_coloring_immediately() {
        let md = "前言一段，带 `行内代码`。\n\n```rust\nlet s = \"hi\";\nfn main() {}\n```\n\n后记。\n";
        let es = vec![Entry::Assistant {
            content: md.into(),
            usage: None,
        }];
        let t = t();
        let eager = render_group(&es, Range { start: 0, end: 1 }, &t, true, true, 40, false);
        let mut late = render_group(&es, Range { start: 0, end: 1 }, &t, true, true, 40, true);

        assert_eq!(late.height, eager.height, "推迟上色不许改高度");
        assert_eq!(rows_text(&late), rows_text(&eager), "推迟上色不许改文本");
        assert_eq!(late.pending_len(), 1, "围栏该登记成一个待上色段");
        assert_ne!(cells(&late), cells(&eager), "推迟的那份该还没上色");

        assert_eq!(late.color_rows(0, late.height, &t), 1, "该补上一段");
        assert_eq!(late.pending_len(), 0, "补过的段不该留着");
        assert_eq!(late.height, eager.height);
        assert_eq!(
            cells(&late),
            cells(&eager),
            "补完色该与急切渲染逐格相同"
        );
    }

    /// 只补落在给出行区间里的段：视口之外的那些留着待办（滚到再补）。
    #[test]
    fn coloring_one_row_range_leaves_the_other_fences_pending() {
        let md = "```rust\nlet a = 1;\n```\n\n中间正文。\n\n```python\nx = 1\n```\n";
        let es = vec![Entry::Assistant {
            content: md.into(),
            usage: None,
        }];
        let t = t();
        let mut b = render_group(&es, Range { start: 0, end: 1 }, &t, true, true, 40, true);
        assert_eq!(b.pending_len(), 2, "两个围栏 = 两段");
        // 只给头 3 行（第一个围栏所在）补色。
        assert_eq!(b.color_rows(0, 3, &t), 1);
        assert_eq!(b.pending_len(), 1, "下面那段该还留着");
    }

    #[test]
    fn block_rows_fit_the_width_exactly() {
        let es = vec![user(
            "一段很长很长的中文消息用来测试折行 behaviour with mixed 文本 abcdefgh",
        )];
        let b = render_group(&es, Range { start: 0, end: 1 }, &t(), true, false, 20, false);
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
        let b = render_group(&es, Range { start: 0, end: 1 }, &t(), true, false, 10, false);
        let joined: String = b
            .rows
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert_eq!(joined.replace(['▌', ' '], ""), text, "折行不得丢字");
    }
}
