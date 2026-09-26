//! TUI layer — ratatui + crossterm.
//!
//! Division of labor:
//! - 捕捉（零语义）：`keys`
//! - 纯逻辑（不碰终端、可单测）：`text` / `theme`
//! - 编排：`app`（所有权账本）/ `session`（事件循环接线）
//! - 界面全部归 `zone`：APP → Zone → 子区 → 各自渲染自己。
//!   主区三个子区（history / input / reserved）各自拥有一切私有状态，
//!   几何算术在主区共用的 `zone::main::geometry`。
pub mod app;
pub mod keys;
pub mod session;
pub mod text;
pub mod theme;
pub mod zone;

/// Render a transcript slice through the real pipeline (markdown,
/// syntect, theme tokens) for docs/preview tooling. Documented, hidden
/// from the API surface intent: examples/render_preview.rs is the only
/// consumer.
pub fn render_transcript_public(
    entries: &[crate::server::entry::Entry],
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
) -> Vec<ratatui::text::Line<'static>> {
    let p = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
    let mut out = Vec::new();
    for r in zone::main::history::render::blocks::blocks(entries) {
        if !out.is_empty() {
            out.push(ratatui::text::Line::from(""));
        }
        out.extend(zone::main::history::render::chat::render_at_public(
            &entries[r.start..r.end],
            &p,
            show_reasoning,
            tools_expanded,
            width,
        ));
    }
    out
}

/// Bench harness surface (doc-hidden): BlockCache + blocks() re-exported
/// for examples/render_bench.rs. Not API.
#[doc(hidden)]
pub mod bench {
    use crate::server::entry::Entry;
    use crate::tui::zone::main::history::cache::Item;

    pub use crate::tui::zone::main::history::cache::BlockCache;
    pub use crate::tui::zone::main::history::render::blocks::blocks;

    /// 块清单：`blocks()` 分好组，键 = 序号（bench 里没有库里的 id）。
    fn items_of<'a>(entries: &'a [Entry], ranges: &[crate::grouping::Range]) -> Vec<Item<'a>> {
        ranges
            .iter()
            .enumerate()
            .map(|(i, r)| Item {
                key: i as i64 + 1,
                entries: &entries[r.start..r.end],
            })
            .collect()
    }

    /// Cache sync + a bottom-anchored window render, mirroring the history
    /// zone's hot path. `offset_rows` counts up from the transcript bottom
    /// (0 = newest).
    pub fn window_bottom(
        cache: &mut BlockCache,
        entries: &[Entry],
        offset_rows: usize,
        viewport_rows: usize,
        width: usize,
    ) -> (Vec<ratatui::text::Line<'static>>, usize, usize) {
        let p = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        let ranges = blocks(entries);
        let items = items_of(entries, &ranges);
        cache.sync(&items, 0, width);
        // 从末尾往回攒 `offset + viewport` 行，再往下丢掉 `offset` 行。
        let need = offset_rows.saturating_add(viewport_rows);
        let mut below = 0usize;
        let mut picked: Vec<usize> = Vec::new();
        for i in (0..items.len()).rev() {
            let h = cache.height(&items[i], &p, true, true);
            if h == 0 {
                continue;
            }
            below += h + 1;
            picked.push(i);
            if below >= need {
                break;
            }
        }
        picked.reverse();
        let mut rows = cache.rows_for(&items, &picked, &p, true, true);
        rows.truncate(rows.len().saturating_sub(offset_rows));
        let drop = rows.len().saturating_sub(viewport_rows);
        rows.drain(..drop);
        (rows, cache.cached_rows(), cache.cached_blocks())
    }

    /// Direct block-range window (for cache-hit benchmarks).
    pub fn window_at(
        cache: &mut BlockCache,
        entries: &[Entry],
        b0: usize,
        b1: usize,
        width: usize,
    ) -> (Vec<ratatui::text::Line<'static>>, usize, usize) {
        let p = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        let ranges = blocks(entries);
        let items = items_of(entries, &ranges);
        cache.sync(&items, 0, width);
        let picked: Vec<usize> = (b0..b1.min(items.len())).collect();
        let rows = cache.rows_for(&items, &picked, &p, true, true);
        (rows, cache.cached_rows(), cache.cached_blocks())
    }

    pub fn block_count(entries: &[Entry]) -> usize {
        blocks(entries).len()
    }

    /// 把一串条目包成一条 `transcript` 快照消息（键 = 序号），给 bench 的
    /// "装一条转录"用。
    pub fn transcript_of(entries: Vec<Entry>) -> crate::server::wire::ServerMsg {
        let blocks = crate::grouping::chunks(&entries)
            .into_iter()
            .enumerate()
            .map(|(i, r)| crate::server::wire::WireBlock {
                id: i as i64 + 1,
                entries: entries[r.start..r.end].to_vec(),
            })
            .collect();
        crate::server::wire::ServerMsg::Transcript {
            blocks,
            live: Vec::new(),
        }
    }

    /// Measure-only window walk (no rows painted): for bench offsets.
    /// Returns the block range `[b0, b1)` covering `[offset, offset+viewport)`.
    pub fn walk_window(
        cache: &mut BlockCache,
        entries: &[Entry],
        offset_rows: usize,
        viewport_rows: usize,
        width: usize,
    ) -> (usize, usize) {
        let p = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        let ranges = blocks(entries);
        let items = items_of(entries, &ranges);
        cache.sync(&items, 0, width);
        let need = offset_rows.saturating_add(viewport_rows);
        let mut below = 0usize;
        let mut top: Option<usize> = None;
        let mut bottom: Option<usize> = None;
        for i in (0..items.len()).rev() {
            let h = cache.height(&items[i], &p, true, true);
            if h == 0 {
                continue;
            }
            if bottom.is_none() && offset_rows < below + h + 1 {
                bottom = Some(i);
            }
            if below + h >= need {
                top = Some(i);
                break;
            }
            below += h + 1;
        }
        let (b_bottom, b_top) = (bottom.unwrap_or(0), top.unwrap_or(0));
        (b_top.min(b_bottom), b_top.max(b_bottom) + 1)
    }
}
