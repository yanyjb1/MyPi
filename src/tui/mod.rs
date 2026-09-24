//! TUI layer — ratatui + crossterm.
//!
//! Division of labor (aligned with pi's packages/tui + modes/interactive):
//! - Pure logic (no terminal dependency, unit-testable):
//!   `text` / `editor` / `keys` / `theme` / `layout` /
//!   `undo` / `history` / `path` (including the completion popup state machine)
//! - Components (render state into Lines): `components/*`
//! - Orchestration (event loop + rendering): `app` / `view` / `events`
pub mod app;
pub mod chat;
pub mod completion;
pub mod components;
pub mod editor;
pub mod highlight;
pub mod keys;
pub mod layout;
pub mod session;
pub mod text;
pub mod theme;
pub mod transcript;
pub mod view;
pub mod zones;
pub mod zones_impl;

/// Render a transcript slice through the real pipeline (markdown,
/// syntect, theme tokens) for docs/preview tooling. Documented, hidden
/// from the API surface intent: examples/render_preview.rs is the only
/// consumer.
pub fn render_transcript_public(
    entries: &[crate::entry::Entry],
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
) -> Vec<ratatui::text::Line<'static>> {
    let p = crate::tui::theme::Palette::current();
    let mut out = Vec::new();
    for r in transcript::blocks::blocks(entries) {
        if !out.is_empty() {
            out.push(ratatui::text::Line::from(""));
        }
        out.extend(transcript::components::chat::render_at_public(
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
    pub use crate::tui::transcript::blocks::blocks;
    pub use crate::tui::transcript::cache::BlockCache;

    /// Cache sync + windowed render, mirroring view.rs's hot path.
    /// `offset_rows` counts up from the transcript bottom (0 = newest).
    pub fn window_bottom(
        cache: &mut BlockCache,
        entries: &[crate::entry::Entry],
        offset_rows: usize,
        viewport_rows: usize,
        width: usize,
    ) -> (Vec<ratatui::text::Line<'static>>, usize, usize) {
        let p = crate::tui::theme::Palette::current();
        cache.sync(entries, 0, &p, true, true, width);
        let (b0, b1) =
            cache.window_from_bottom(entries, &p, true, true, offset_rows, viewport_rows);
        let rows = cache.rows_for(entries, &p, true, true, b0..b1);
        (rows, cache.cached_rows(), cache.cached_blocks())
    }

    /// Old-style direct block-range window (for cache-hit benchmarks).
    pub fn window_at(
        cache: &mut BlockCache,
        entries: &[crate::entry::Entry],
        b0: usize,
        b1: usize,
        width: usize,
    ) -> (Vec<ratatui::text::Line<'static>>, usize, usize) {
        let p = crate::tui::theme::Palette::current();
        cache.sync(entries, 0, &p, true, true, width);
        let rows = cache.rows_for(entries, &p, true, true, b0..b1);
        (rows, cache.cached_rows(), cache.cached_blocks())
    }

    pub fn block_count(entries: &[crate::entry::Entry]) -> usize {
        blocks(entries).len()
    }
    /// Measure-only window walk (no rows painted): for bench offsets.
    pub fn walk_window(
        cache: &mut BlockCache,
        entries: &[crate::entry::Entry],
        offset_rows: usize,
        viewport_rows: usize,
        width: usize,
    ) -> (usize, usize) {
        let p = crate::tui::theme::Palette::current();
        cache.sync(entries, 0, &p, true, true, width);
        cache.window_from_bottom(entries, &p, true, true, offset_rows, viewport_rows)
    }
}
