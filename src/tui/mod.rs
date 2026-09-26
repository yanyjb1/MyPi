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
    pub use crate::tui::zone::main::history::render::blocks::blocks;
    pub use crate::tui::zone::main::history::cache::BlockCache;

    /// Cache sync + windowed render, mirroring view.rs's hot path.
    /// `offset_rows` counts up from the transcript bottom (0 = newest).
    pub fn window_bottom(
        cache: &mut BlockCache,
        entries: &[crate::server::entry::Entry],
        offset_rows: usize,
        viewport_rows: usize,
        width: usize,
    ) -> (Vec<ratatui::text::Line<'static>>, usize, usize) {
        let p = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        cache.sync(entries, 0, &p, true, true, width);
        let w = cache.window_from_bottom(entries, &p, true, true, offset_rows, viewport_rows);
        let rows = cache.rows_for(entries, &p, true, true, w.b0..w.b1);
        (rows, cache.cached_rows(), cache.cached_blocks())
    }

    /// Old-style direct block-range window (for cache-hit benchmarks).
    pub fn window_at(
        cache: &mut BlockCache,
        entries: &[crate::server::entry::Entry],
        b0: usize,
        b1: usize,
        width: usize,
    ) -> (Vec<ratatui::text::Line<'static>>, usize, usize) {
        let p = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        cache.sync(entries, 0, &p, true, true, width);
        let rows = cache.rows_for(entries, &p, true, true, b0..b1);
        (rows, cache.cached_rows(), cache.cached_blocks())
    }

    pub fn block_count(entries: &[crate::server::entry::Entry]) -> usize {
        blocks(entries).len()
    }
    /// Measure-only window walk (no rows painted): for bench offsets.
    pub fn walk_window(
        cache: &mut BlockCache,
        entries: &[crate::server::entry::Entry],
        offset_rows: usize,
        viewport_rows: usize,
        width: usize,
    ) -> (usize, usize) {
        let p = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        cache.sync(entries, 0, &p, true, true, width);
        let w = cache.window_from_bottom(entries, &p, true, true, offset_rows, viewport_rows);
        (w.b0, w.b1)
    }
}
