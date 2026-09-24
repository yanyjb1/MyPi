//! TUI layer — ratatui + crossterm.
//!
//! Division of labor (aligned with pi's packages/tui + modes/interactive):
//! - Pure logic (no terminal dependency, unit-testable):
//!   `text` / `editor` / `keys` / `theme` / `layout` /
//!   `undo` / `history` / `path` (including the completion popup state machine)
//! - Components (render state into Lines): `components/*`
//! - Orchestration (event loop + rendering): `app` / `view` / `events`
pub mod app;
mod transcript;
pub mod completion;
pub mod chat;
pub mod editor;
pub mod components;
pub mod highlight;
pub mod keys;
pub mod layout;
pub mod text;
pub mod session;
pub mod theme;
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
