//! The task list, pinned under the transcript.

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::ui::tui::TodoFold;
use crate::ui::tui::text::card::clip_spans;
use crate::ui::tui::text::glyphs::{TODO_DONE, TODO_PENDING};
use crate::ui::tui::theme::{HistoryTheme, Token};

/// One line of the model's checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub text: String,
    pub done: bool,
}

/// The pinned rows: a title, then the open items, capped by the fold policy.
pub(in crate::ui::tui::zone::main::history) fn rows(
    tasks: &[Task],
    fold: TodoFold,
    width: usize,
    t: &HistoryTheme,
    budget: usize,
) -> Vec<Line<'static>> {
    if tasks.is_empty() || budget < 2 {
        return Vec::new();
    }
    let clip = |line: Line<'static>| -> Line<'static> {
        if line.width() <= width {
            return line;
        }
        Line::from(clip_spans(line.spans, width))
    };
    let mut out = vec![clip(Line::from(t.fg_mod(
        "TODO",
        Token::ToolTitle,
        Modifier::BOLD,
    )))];
    let room = usize::from(budget).saturating_sub(1);
    let cap = fold.cap().min(room);
    for task in tasks.iter().filter(|task| !task.done).take(cap) {
        let (glyph, tok) = (TODO_PENDING, Token::Dim);
        out.push(clip(Line::from(vec![
            Span::raw(format!("{glyph} ")),
            t.fg(task.text.clone(), tok),
        ])));
    }
    let open = tasks.iter().filter(|task| !task.done).count();
    if open > cap {
        out.push(clip(Line::from(
            t.fg(format!("… {} more", open - cap), Token::Dim),
        )));
    }
    let done = tasks.iter().filter(|task| task.done).count();
    if done > 0 && out.len() < usize::from(budget) {
        out.push(clip(Line::from(
            t.fg(format!("{TODO_DONE} {done} done"), Token::Dim),
        )));
    }
    out
}
