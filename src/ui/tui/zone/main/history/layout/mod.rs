//! Re-layout a split product at a new width. No raw material is requested here.

pub mod lazy;
pub mod visible;

use ratatui::text::Line;

use super::product::{CutPoint, Product};
use super::render::block::{Block, Pending, wrap_rows};
use super::render::text::highlight;
use super::render::theme::HistoryTheme;

pub fn layout(product: &Product, width: usize, _seam: Option<CutPoint>) -> Block {
    let lines: Vec<Line<'static>> = product.lines().cloned().collect();
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    let mut it = lines.into_iter();
    let mut consumed = 0usize;

    for deferred in &product.deferred {
        let gap = deferred.before.saturating_sub(consumed);
        rows.extend(wrap_rows(it.by_ref().take(gap), width));
        consumed += gap;
        let lo = rows.len();
        let mut line_rows: Vec<std::ops::Range<usize>> = Vec::with_capacity(deferred.count);
        for _ in 0..deferred.count {
            let Some(line) = it.next() else { break };
            let before = rows.len();
            rows.extend(wrap_rows(std::iter::once(line), width));
            line_rows.push(before..rows.len());
        }
        consumed += deferred.count;
        if let Some(cursor) =
            highlight::HighlightCursor::new(&deferred.code, deferred.lang.as_deref(), t)
        {
            pending.push(Pending {
                rows: lo..rows.len(),
                total: line_rows.len(),
                line_rows,
                painted: 0,
                cursor,
            });
        }
    }
    rows.extend(wrap_rows(it.by_ref(), width));
    Block {
        height: rows.len(),
        rows,
        width,
        pending,
    }
}
