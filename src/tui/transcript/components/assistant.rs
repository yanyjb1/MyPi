//! Assistant node: the model's answer alone. The thinking chain is its
//! own entry/block (`Entry::Reasoning`, styled in `chat.rs`) — one block
//! per display unit, so no block ever renders in two forms.

use ratatui::text::Line;

use crate::entry::UsageSummary;
use crate::tui::theme::Palette;

// Model reply: plain foreground, no background.
pub(super) fn assistant_block(
    content: &str,
    usage: Option<&UsageSummary>,
    p: &Palette,
) -> Vec<Line<'static>> {
    // usage is persisted with the entry (billing/stats) and never rendered as fine print in chat
    let _ = usage;
    crate::tui::components::markdown::render_markdown(content, p)
}
