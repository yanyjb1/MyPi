//! The ESC row: what is running, how long it has run, and the accent on its label.

use std::time::Duration;

use super::super::super::row::{Lead, Row, Run};
use super::sweep;
use crate::ui::tui::text::duration;
use crate::ui::tui::text::glyphs as g;
use crate::ui::tui::theme::fixed;

/// `spinner ESC <elapsed> · <label>`, the label swept by the accent.
pub(crate) fn row(spinner: char, elapsed: Duration, label: &str, phase: usize) -> Row {
    let mut runs = vec![
        Run::fixed(spinner.to_string(), fixed::DIM),
        Run::fixed(format!(" {} ", g::ESC), fixed::DIM),
        Run::fixed(duration(elapsed.as_millis() as u64), fixed::DIM),
    ];
    let label = label.trim();
    if !label.is_empty() {
        runs.push(Run::fixed(format!(" {} ", g::SEP), fixed::DIM));
        runs.extend(sweep::runs(label, phase));
    }
    Row::of(Lead::None, runs)
}
