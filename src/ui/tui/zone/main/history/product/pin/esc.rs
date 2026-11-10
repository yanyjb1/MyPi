//! The ESC row: what is running, how long it has run, and the one token that shimmers.

use std::time::Duration;

use super::super::super::row::{Lead, Row, Run};
use super::sweep;
use crate::ui::tui::text::duration;
use crate::ui::tui::text::glyphs as g;
use crate::ui::tui::theme::fixed;

/// Columns the sweep walks per second, whatever the stream does.
const SWEEP_CELLS_PER_SEC: u128 = 30;

/// `spinner ESC <elapsed> · <label>`; thinking sweeps the label, a call sweeps `ESC`.
pub(crate) fn row(spinner: char, elapsed: Duration, label: &str, thinking: bool) -> Row {
    let millis = elapsed.as_millis();
    let phase = (millis * SWEEP_CELLS_PER_SEC / 1000) as usize;
    let mut runs = vec![
        Run::fixed(spinner.to_string(), fixed::DIM),
        Run::fixed(" ".to_string(), fixed::DIM),
    ];
    if thinking {
        runs.push(Run::accent(g::ESC));
    } else {
        runs.extend(sweep::runs(g::ESC, phase));
    }
    runs.push(Run::fixed(
        format!(" {} ", duration(millis as u64)),
        fixed::DIM,
    ));
    let label = label.trim();
    if !label.is_empty() {
        runs.push(Run::fixed(format!(" {} ", g::SEP), fixed::DIM));
        if thinking {
            runs.extend(sweep::runs(label, phase));
        } else {
            runs.push(Run::accent(label));
        }
    }
    Row::of(Lead::None, runs)
}
