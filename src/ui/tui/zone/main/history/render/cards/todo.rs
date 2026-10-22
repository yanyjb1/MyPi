//! The task list, drawn as a tree and pinned under the transcript.

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use super::super::text::glyphs::{
    TODO_ACTIVE, TODO_BLOCKED, TODO_DONE, TODO_DROPPED, TODO_PENDING, TREE_BRANCH, TREE_HOOK,
    TREE_LAST, TREE_VERTICAL,
};
use super::super::theme::{HistoryTheme, Token};
use super::clip_spans;
use crate::server::agent::tools::TodoPhase;
use crate::server::agent::tools::todo::state::{TodoStatus, TodoTask};
use crate::server::config::read::config::TodoFold;

pub fn roman(one_based: usize) -> String {
    const PAIRS: [(usize, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut n = one_based;
    let mut out = String::new();
    for (v, sym) in PAIRS {
        while n >= v {
            out.push_str(sym);
            n -= v;
        }
    }
    out
}

pub fn rows(
    phases: &[TodoPhase],
    touched: &[String],
    fold: TodoFold,
    width: usize,
    t: &HistoryTheme,
    budget: usize,
) -> Vec<Line<'static>> {
    if phases.is_empty() || budget == 0 {
        return Vec::new();
    }
    if phases
        .iter()
        .flat_map(|p| p.tasks.iter())
        .all(|x| x.status.is_closed())
    {
        return Vec::new();
    }
    let folding = fold.is_on() && phases.len() > 1 && !touched.is_empty();
    let cap = fold.cap();
    let clip = |line: Line<'static>| -> Line<'static> {
        if line.width() <= width {
            return line;
        }
        let style = line.style;
        Line::from(clip_spans(line.spans, width)).style(style)
    };
    let title = t.fg_mod("TODO", Token::ToolTitle, Modifier::BOLD);
    if budget < 3 {
        return vec![clip(Line::from(title))];
    }
    let mut body: Vec<Line<'static>> = Vec::new();
    let total = phases.len();
    for (idx, phase) in phases.iter().enumerate() {
        let last_phase = idx + 1 == total;
        let branch = if last_phase { TREE_LAST } else { TREE_BRANCH };
        let name = format!("{}. {}", roman(idx + 1), phase.name);
        let done = phase.tasks.iter().filter(|x| x.status.is_closed()).count();
        let progress = format!(" · {done}/{}", phase.tasks.len());
        let live: Vec<_> = phase
            .tasks
            .iter()
            .filter(|x| !x.status.is_closed())
            .collect();
        let untouched = folding && !touched.iter().any(|n| n == &phase.name);
        let mut spans = vec![t.fg(format!(" {branch} "), Token::Dim)];
        if live.is_empty() || untouched {
            spans.push(t.fg_mod(name, Token::Dim, Modifier::BOLD));
            spans.push(t.fg(progress, Token::Dim));
            body.push(Line::from(spans));
            continue;
        }
        spans.push(t.fg_mod(name, Token::ToolTitle, Modifier::BOLD));
        spans.push(t.fg(progress, Token::Dim));
        body.push(Line::from(spans));
        let shown = phase.tasks.len().min(cap);
        let cont = if last_phase { "" } else { TREE_VERTICAL };
        for ti in 0..shown {
            let task = &phase.tasks[ti];
            let sub = if ti + 1 == shown {
                TREE_LAST
            } else {
                TREE_BRANCH
            };
            let mut line = vec![t.fg(format!(" {cont}  {sub} "), Token::Dim)];
            line.extend(task_spans(t, task));
            body.push(Line::from(line));
        }
        if shown < phase.tasks.len() {
            let hidden = phase.tasks.len() - shown;
            body.push(Line::from(vec![
                t.fg(format!(" {cont}  "), Token::Dim),
                t.fg(format!("… 还有 {hidden} 条"), Token::Muted),
            ]));
        }
    }
    body.push(Line::from(
        t.fg(format!(" {TREE_HOOK}{}", "-".repeat(3)), Token::Dim),
    ));

    let mut out = vec![Line::from(title)];
    let room = budget - 1;
    if body.len() <= room {
        out.extend(body);
        return out.into_iter().map(clip).collect();
    }
    let hidden = body.len() - (room - 1);
    out.extend(body.into_iter().take(room - 1));
    out.push(Line::from(
        t.fg(format!(" … 还有 {hidden} 行"), Token::Muted),
    ));
    out.into_iter().map(clip).collect()
}

fn task_spans(t: &HistoryTheme, task: &TodoTask) -> Vec<Span<'static>> {
    let (glyph, tok, strike) = match task.status {
        TodoStatus::Done => (TODO_DONE, Token::Dim, true),
        TodoStatus::InProgress => (TODO_ACTIVE, Token::ToolEdgePending, false),
        TodoStatus::Blocked => (TODO_BLOCKED, Token::Warn, false),
        TodoStatus::Abandoned => (TODO_DROPPED, Token::Err, true),
        TodoStatus::Pending => (TODO_PENDING, Token::AssistantText, false),
    };
    let mut style = ratatui::style::Style::new().fg(t.get(tok));
    if strike {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    let mut out = vec![Span::styled(format!("{glyph} "), style)];
    out.push(Span::styled(task.content.clone(), style));
    if let Some(why) = &task.blocker {
        out.push(t.fg(format!("（{why}）"), Token::Warn));
    }
    out
}
