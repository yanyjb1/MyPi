//! The task list, pinned under the transcript.

use super::super::super::row::{Ink, Lead, Row, Run};
use crate::ui::tui::TodoFold;
use crate::ui::tui::text::glyphs::{TODO_DONE, TODO_PENDING};
use crate::ui::tui::theme::fixed;

/// One line of the model's checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub text: String,
    pub done: bool,
}

/// A title, the open items, then a done count. Capped by the fold policy.
pub(crate) fn rows(tasks: &[Task], fold: TodoFold, width: usize, budget: usize) -> Vec<Row> {
    if tasks.is_empty() || budget < 2 {
        return Vec::new();
    }
    let mut out = vec![clip(
        Row::of(
            Lead::None,
            vec![Run::fixed("TODO", fixed::TOOL_TITLE).bold()],
        ),
        width,
    )];
    let room = budget.saturating_sub(1);
    let cap = fold.cap().min(room);
    for task in tasks.iter().filter(|task| !task.done).take(cap) {
        out.push(clip(
            Row::of(
                Lead::None,
                vec![
                    Run::plain(format!("{TODO_PENDING} ")),
                    Run::fixed(task.text.clone(), fixed::DIM),
                ],
            ),
            width,
        ));
    }
    let open = tasks.iter().filter(|task| !task.done).count();
    if open > cap {
        out.push(clip(
            Row::text(
                Lead::None,
                format!("… {} more", open - cap),
                Some(Ink::Fixed(fixed::DIM)),
            ),
            width,
        ));
    }
    let done = tasks.iter().filter(|task| task.done).count();
    if done > 0 && out.len() < budget {
        out.push(clip(
            Row::text(
                Lead::None,
                format!("{TODO_DONE} {done} done"),
                Some(Ink::Fixed(fixed::DIM)),
            ),
            width,
        ));
    }
    out
}

/// Trim a row to the width the terminal gave us.
fn clip(mut row: Row, width: usize) -> Row {
    let mut used = 0usize;
    let mut keep = 0usize;
    for run in &row.runs {
        if used >= width {
            break;
        }
        keep += 1;
        used += run.width();
    }
    row.runs.truncate(keep);
    if used <= width {
        return row;
    }
    let mut left = width;
    for run in &mut row.runs {
        let mut text = String::new();
        for ch in run.text.chars() {
            let w = crate::ui::tui::text::char_width(ch);
            if w > left {
                break;
            }
            left -= w;
            text.push(ch);
        }
        run.text = text;
    }
    row
}
