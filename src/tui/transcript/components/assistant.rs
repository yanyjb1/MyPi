//! Assistant node: reasoning (muted italic) + answer, with the mandatory
//! blank row between the two thoughts. Restyle the AI's voice here.

use ratatui::style::Modifier;
use ratatui::text::Line;

use crate::entry::UsageSummary;
use crate::tui::theme::Palette;

// Model reply: plain foreground, no background. Reasoning (when shown) is muted.
pub(super) fn assistant_block(
    content: &str,
    reasoning: Option<&str>,
    usage: Option<&UsageSummary>,
    p: &Palette,
    show_reasoning: bool,
) -> Vec<Line<'static>> {
    // usage is persisted with the entry (billing/stats) and never rendered as fine print in chat
    let _ = usage;
    let mut out = Vec::new();
    // Reasoning: the thinking chain, shown by default. An earlier revision hid
    // it unless Ctrl+T was pressed; the toggle now only suppresses it.
    if show_reasoning && let Some(r) = reasoning.filter(|r| !r.trim().is_empty()) {
        for mut line in crate::tui::components::markdown::render_markdown(r, p) {
            for sp in &mut line.spans {
                sp.style = sp.style.fg(p.muted).add_modifier(Modifier::ITALIC);
            }
            out.push(line);
        }
        // Reasoning and the answer are both "the AI's message" but they are
        // two thoughts: the same blank-row separation every other node gets.
        out.push(Line::from(""));
    }
    out.extend(crate::tui::components::markdown::render_markdown(content, p));
    out
}
