//! The checklist as pinned rows: numbered, the last finished item kept for context.

use ratatui::style::Modifier;

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

/// A title, then the window of items the fold policy allows, numbered by position.
pub(crate) fn rows(tasks: &[Task], fold: TodoFold, width: usize) -> Vec<Row> {
    if tasks.is_empty() {
        return Vec::new();
    }
    let mut out = vec![clip(
        Row::of(
            Lead::None,
            vec![Run::fixed("TODO", fixed::TOOL_TITLE).bold()],
        ),
        width,
    )];
    let window = window(tasks);
    let shown = window.len().min(fold.cap());
    let pad = window
        .iter()
        .take(shown)
        .map(|(at, _)| (at + 1).to_string().len())
        .max()
        .unwrap_or(1);
    for (number, task) in window.iter().take(shown) {
        let mut runs = vec![Run::fixed(
            format!(
                "{} {:>pad$}  ",
                if task.done { TODO_DONE } else { TODO_PENDING },
                number + 1
            ),
            fixed::DIM,
        )];
        runs.push(if task.done {
            Run::with(
                task.text.clone(),
                Some(Ink::Fixed(fixed::DIM)),
                Modifier::CROSSED_OUT,
            )
        } else {
            Run::plain(task.text.clone())
        });
        out.push(clip(Row::of(Lead::None, runs), width));
    }
    if window.len() > shown {
        out.push(clip(
            Row::text(
                Lead::None,
                format!("… {} more", window.len() - shown),
                Some(Ink::Fixed(fixed::DIM)),
            ),
            width,
        ));
    }
    out
}

/// The window: the last finished item for context, then every open one in order.
fn window(tasks: &[Task]) -> Vec<(usize, &Task)> {
    let mut out: Vec<(usize, &Task)> = Vec::new();
    if let Some(at) = tasks.iter().rposition(|task| task.done) {
        out.push((at, &tasks[at]));
    }
    out.extend(tasks.iter().enumerate().filter(|(_, task)| !task.done));
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
