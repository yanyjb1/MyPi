//! Prose and every other markdown-class segment: colored rows at a width.

use ratatui::style::Modifier;

use super::fold_row;
use crate::ui::tui::theme::fixed;
use crate::ui::tui::zone::main::history::markdown;
use crate::ui::tui::zone::main::history::product::natural;
use crate::ui::tui::zone::main::history::product::segment::Segment;
use crate::ui::tui::zone::main::history::row::{Ink, Lead, Row, Run};

/// One segment's rows: each natural line colored, then broken at the width, one group per
/// source line.
pub fn rows_grouped(seg: &Segment, width: usize, thinking: bool) -> Vec<Vec<Row>> {
    let mut out: Vec<Vec<Row>> = Vec::new();
    for line in natural::lines(seg) {
        let mut laid = markdown::rows(&line);
        let mut runs = laid.pop().map(|row| row.runs).unwrap_or_default();
        if thinking {
            chain(&mut runs);
        }
        out.push(fold_row(Row::of(Lead::First, runs), width));
    }
    out
}

/// The thinking chain's own look: muted and leaning.
fn chain(runs: &mut [Run]) {
    for run in runs.iter_mut() {
        run.ink = Some(Ink::Fixed(fixed::THINKING));
        run.attrs |= Modifier::ITALIC;
    }
}
