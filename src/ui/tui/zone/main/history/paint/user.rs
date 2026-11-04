//! A user message: a bar on the left, the body on a filled band.

use ratatui::style::Color;

use super::super::layout::line::{Ink, Lead, Row, Run};
use super::super::wire::decode::Block;
use super::prose;
use crate::ui::tui::theme::fixed;

const GUTTER: &str = "▌ ";
const GUTTER_COLS: usize = 2;

pub(crate) fn block(block: &Block, width: usize) -> Vec<Row> {
    let text = block
        .content
        .as_deref()
        .or(block.text.as_deref())
        .unwrap_or("");
    let fill = Ink::Fixed(fixed::USER_BG);
    let avail = width.saturating_sub(GUTTER_COLS);
    let mut out = vec![gutter(fill, width)];
    for row in prose::inline(text, avail) {
        let mut line = Row::new(Lead::None);
        line.push(Run::accent(GUTTER).fill(fill));
        let joined: String = row.runs.iter().map(|run| run.text.as_str()).collect();
        line.push(Run::fixed(joined, body_ink()).fill(fill));
        pad(&mut line, width, fill);
        out.push(line);
    }
    out.push(gutter(fill, width));
    out
}

/// The band is dark, so a terminal-default foreground is raised to white.
fn body_ink() -> Color {
    if fixed::USER_TEXT == Color::Reset {
        Color::White
    } else {
        fixed::USER_TEXT
    }
}

fn gutter(fill: Ink, width: usize) -> Row {
    let mut row = Row::new(Lead::None);
    row.push(Run::accent(GUTTER).fill(fill));
    pad(&mut row, width, fill);
    row
}

fn pad(row: &mut Row, width: usize, fill: Ink) {
    let used: usize = row.runs.iter().map(Run::width).sum();
    if used < width {
        row.push(Run::plain(" ".repeat(width - used)).fill(fill));
    }
}
