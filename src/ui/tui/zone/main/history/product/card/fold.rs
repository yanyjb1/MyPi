//! A long run of rows cut down to a head and a tail.

use crate::ui::tui::theme::fixed;
use crate::ui::tui::zone::main::history::row::{Ink, Lead, Row};

/// Head row, a placeholder, then the newest rows. Fits: untouched.
pub fn fold(rows: Vec<Row>, budget: usize, expanded: bool, running: bool) -> Vec<Row> {
    let budget = budget.max(3);
    if expanded || rows.len() <= budget {
        return rows;
    }
    if running {
        return rows[rows.len() - budget..].to_vec();
    }
    let keep = budget - 2;
    let hidden = rows.len() - 1 - keep;
    let mut out = Vec::with_capacity(budget);
    out.push(rows[0].clone());
    out.push(Row::text(
        Lead::None,
        format!("{hidden} lines earlier, Ctrl + O to expand"),
        Some(Ink::Fixed(fixed::DIM)),
    ));
    out.extend(rows.into_iter().skip(1 + hidden));
    out
}
