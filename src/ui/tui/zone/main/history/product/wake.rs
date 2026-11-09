//! The wake block: one release line, any further lines under it.

use crate::ui::tui::theme::fixed;
use crate::ui::tui::zone::main::history::row::{Ink, Lead, Row, Run};

/// One wake, one release line; a multiline text keeps its lines below.
pub(crate) fn rows(text: &str) -> Vec<Row> {
    let mut lines = text.lines();
    let head = lines.next().unwrap_or_default();
    let mut out = vec![Row::of(
        Lead::None,
        vec![
            Run::fixed("[wake] ", fixed::OK),
            Run::fixed("Kairos-Release", fixed::TOOL_TITLE).bold(),
            Run::fixed(format!(": {head}"), fixed::MUTED),
        ],
    )];
    out.extend(
        lines.map(|line| Row::text(Lead::None, line.to_string(), Some(Ink::Fixed(fixed::MUTED)))),
    );
    out
}
