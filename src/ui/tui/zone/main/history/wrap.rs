//! One natural line to the rows the terminal has room for.

use super::line::{Lead, Row, Run};
use crate::ui::tui::text::display_width;

/// Wrap one natural line. Continuations carry `Lead::Wrapped`; nothing else changes.
pub fn wrap(row: &Row, width: usize) -> Vec<Row> {
    let avail = width.saturating_sub(row.lead.width()).max(1);
    if row.width() <= width {
        return vec![row.clone()];
    }
    let mut out: Vec<Row> = Vec::new();
    let mut cur = Row::new(row.lead);
    let mut used = 0usize;
    for run in &row.runs {
        let mut buf = String::new();
        for ch in run.text.chars() {
            let w = display_width(&ch.to_string());
            if used + w > avail && (used > 0 || !buf.is_empty()) {
                if !buf.is_empty() {
                    cur.push(Run::with(std::mem::take(&mut buf), run.ink, run.attrs));
                }
                out.push(std::mem::replace(&mut cur, Row::new(next_lead(row.lead))));
                used = 0;
            }
            buf.push(ch);
            used += w;
        }
        if !buf.is_empty() {
            cur.push(Run::with(buf, run.ink, run.attrs));
        }
    }
    if !cur.runs.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(row.clone());
    }
    out
}

fn next_lead(lead: Lead) -> Lead {
    match lead {
        Lead::None => Lead::None,
        Lead::First | Lead::Wrapped => Lead::Wrapped,
    }
}

/// Every natural line of a run of rows, wrapped in order.
pub fn rows(rows: Vec<Row>, width: usize) -> Vec<Row> {
    let mut out = Vec::new();
    for row in rows {
        out.extend(wrap(&row, width));
    }
    out
}
