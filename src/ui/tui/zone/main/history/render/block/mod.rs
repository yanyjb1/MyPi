//! One rendered node: wrapped rows, an exact height, and the fences still to color.

use ratatui::text::Line;

use super::cards::chat;
use super::text::highlight;
use super::theme::HistoryTheme;
use crate::server::storage::BlockKind;
use crate::ui::session::BlockRef;

#[derive(Debug, Clone)]
pub struct Deferred {
    pub(crate) before: usize,
    pub(crate) count: usize,
    pub(crate) code: String,
    pub(crate) lang: Option<String>,
}

pub(crate) struct Pending {
    pub(crate) rows: std::ops::Range<usize>,
    pub(crate) cursor: highlight::HighlightCursor,
    pub(crate) total: usize,
    pub(crate) line_rows: Vec<std::ops::Range<usize>>,
    pub(crate) painted: usize,
}

const HIGHLIGHT_LINES_PER_FRAME: usize = 64;

const HIGHLIGHT_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);

const WIDE_LINE_CHARS: usize = 2000;

pub(crate) struct Block {
    pub(crate) rows: Vec<Line<'static>>,
    pub(crate) height: usize,
    pub(crate) width: usize,
    pub(crate) pending: Vec<Pending>,
}

impl Block {
    pub(crate) fn color_rows(&mut self, lo: usize, hi: usize, _t: &HistoryTheme) -> usize {
        if self.pending.is_empty() || hi <= lo {
            return 0;
        }
        let mut colored = 0usize;
        let mut still: Vec<Pending> = Vec::new();
        for mut p in std::mem::take(&mut self.pending).into_iter().rev() {
            if p.rows.start >= hi || p.rows.end <= lo {
                still.push(p);
                continue;
            }
            let mut spent_lines = 0usize;
            let t0 = std::time::Instant::now();
            while spent_lines < HIGHLIGHT_LINES_PER_FRAME
                && p.painted < p.total
                && t0.elapsed() < HIGHLIGHT_BUDGET
            {
                let target = p.line_rows[p.painted].clone();
                if target.end <= lo || target.start >= hi {
                    if p.cursor.peek_len() > WIDE_LINE_CHARS {
                        break;
                    }
                    let batch = highlight::HighlightCursor::take(&mut p.cursor, 1);
                    if batch.is_empty() {
                        break;
                    }
                    p.painted += 1;
                    spent_lines += 1;
                    continue;
                }
                let batch = highlight::HighlightCursor::take(&mut p.cursor, 1);
                if batch.is_empty() {
                    break;
                }
                let wrapped = wrap_rows(batch.into_iter(), self.width);
                if wrapped.len() == target.len() {
                    self.rows[target].clone_from_slice(&wrapped);
                } else {
                    debug_assert!(
                        wrapped.len() == target.len(),
                        "上色改变了行数：{} vs {}",
                        wrapped.len(),
                        target.len()
                    );
                    p.painted = p.total;
                    break;
                }
                p.painted += 1;
                spent_lines += 1;
            }
            if p.painted >= p.total {
                colored += 1; // 整段补完，从待办里摘掉
            } else {
                still.push(p);
            }
        }
        still.reverse();
        self.pending = still;
        colored
    }
}

pub(crate) fn pair(blocks: &[BlockRef<'_>], at: usize) -> Option<BlockRef<'_>> {
    let first = blocks.get(at)?;
    if first.kind() != BlockKind::ToolRequest {
        return None;
    }
    let second = *blocks.get(at + 1)?;
    if second.kind() != BlockKind::ToolResult {
        return None;
    }
    let id = first.call_id()?;
    (second.call_id().as_deref() == Some(id.as_str())).then_some(second)
}

pub(crate) fn render_product(
    block: BlockRef<'_>,
    paired: Option<BlockRef<'_>>,
    t: &HistoryTheme,
    thinking: bool,
    tool: bool,
    placeholder: &str,
    details: Option<&serde_json::Value>,
    width: usize,
    defer: bool,
    cwd: &str,
) -> super::super::product::Product {
    let (lines, deferred) = chat::single_node(
        block,
        paired,
        t,
        thinking,
        tool,
        placeholder,
        details,
        width,
        defer,
        cwd,
    );
    super::super::product::Product::from_lines(lines, deferred)
}

pub(crate) fn render_group(
    block: BlockRef<'_>,
    paired: Option<BlockRef<'_>>,
    t: &HistoryTheme,
    thinking: bool,
    tool: bool,
    placeholder: &str,
    details: Option<&serde_json::Value>,
    width: usize,
    defer: bool,
    cwd: &str,
) -> Block {
    let product = render_product(
        block,
        paired,
        t,
        thinking,
        tool,
        placeholder,
        details,
        width,
        defer,
        cwd,
    );
    super::super::layout::layout(&product, width, None)
}

pub(crate) fn wrap_rows(
    lines: impl Iterator<Item = Line<'static>>,
    width: usize,
) -> Vec<Line<'static>> {
    let w = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        let total: usize = line
            .spans
            .iter()
            .map(|s| crate::ui::tui::text::display_width(&s.content))
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
                let cw = crate::ui::tui::text::display_width(&ch.to_string());
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

pub(crate) fn block_gap() -> Line<'static> {
    Line::from("")
}
