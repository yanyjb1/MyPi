//! edit: the replaced text as it streams, or the numbered diff of what changed.

use std::ops::Range;

use ratatui::style::Modifier;

use super::super::super::row::{Ink, Lead, Row, Run};
use super::{
    Numbered, Section, ToolBlock, ToolOutcome, ToolRenderer, ToolView, error_body, gutter,
    indentation, output_lines, partial, path_of,
};
use crate::ui::tui::theme::fixed;

pub(crate) struct Edit;

impl ToolRenderer for Edit {
    fn path(&self, b: &ToolBlock<'_>) -> Option<String> {
        path_of(b)
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        edit_call(b)
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        vec![Section {
            label: None,
            lines: edit_result(b, o),
        }]
    }
}

/// The call as it arrives: the text going, then the text coming.
fn edit_call(b: &ToolBlock<'_>) -> Vec<Row> {
    if b.result.is_some() {
        return Vec::new();
    }
    let old = partial::text(b.args, "old").filter(|text| !text.is_empty());
    let new = partial::text(b.args, "new").filter(|text| !text.is_empty());
    let changed = match (old.as_deref(), new.as_deref()) {
        (Some(old), Some(new)) => pair_spans(old, new),
        _ => None,
    };
    let mut out = Vec::new();
    if let Some(old) = old.as_deref() {
        push_text(
            &mut out,
            '-',
            old,
            changed.as_ref().map(|(removed, _)| removed.clone()),
        );
    }
    if let Some(new) = new.as_deref() {
        push_text(&mut out, '+', new, changed.map(|(_, added)| added));
    }
    out
}

/// Text that arrived whole: every line signed, without a number.
fn push_text(out: &mut Vec<Row>, sign: char, text: &str, span: Option<Range<usize>>) {
    for (i, line) in text.lines().enumerate() {
        let span = if i == 0 { span.clone() } else { None };
        out.push(diff_row(sign, None, line, span, 0));
    }
}

/// One diff row: the sign and the number, the indentation, then the text.
fn diff_row(
    sign: char,
    at: Option<usize>,
    text: &str,
    span: Option<Range<usize>>,
    width: usize,
) -> Row {
    let ink = if sign == '-' {
        fixed::DIFF_REMOVED
    } else {
        fixed::DIFF_ADDED
    };
    let mut runs = vec![Run::fixed(gutter(sign, at, width), ink)];
    let (glyphs, body) = indentation(text);
    if !glyphs.is_empty() {
        runs.push(Run::with(glyphs, Some(Ink::Fixed(ink)), Modifier::DIM));
    }
    let span = span.filter(|span| span.start < span.end && span.end <= body.len());
    if let Some(span) = span {
        if span.start > 0 {
            runs.push(Run::fixed(&body[..span.start], ink));
        }
        runs.push(Run::with(
            &body[span.start..span.end],
            Some(Ink::Fixed(ink)),
            Modifier::REVERSED,
        ));
        if span.end < body.len() {
            runs.push(Run::fixed(&body[span.end..], ink));
        }
    } else if !body.is_empty() {
        runs.push(Run::fixed(body, ink));
    }
    Row::of(Lead::None, runs)
}

/// The two single lines' changed runs; the indentation is never part of them.
fn pair_spans(old: &str, new: &str) -> Option<(Range<usize>, Range<usize>)> {
    if old.lines().count() != 1 || new.lines().count() != 1 {
        return None;
    }
    let old = indentation(old.lines().next().unwrap_or_default()).1;
    let new = indentation(new.lines().next().unwrap_or_default()).1;
    changed_spans(&old, &new)
}

/// The run one line has before and after its common prefix and suffix.
fn changed_spans(old: &str, new: &str) -> Option<(Range<usize>, Range<usize>)> {
    if old == new {
        return None;
    }
    let oc: Vec<char> = old.chars().collect();
    let nc: Vec<char> = new.chars().collect();
    let mut pre = 0;
    while pre < oc.len() && pre < nc.len() && oc[pre] == nc[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < oc.len() - pre
        && suf < nc.len() - pre
        && oc[oc.len() - 1 - suf] == nc[nc.len() - 1 - suf]
    {
        suf += 1;
    }
    let at = |s: &str, ci: usize| s.char_indices().nth(ci).map_or(s.len(), |(b, _)| b);
    Some((
        at(old, pre)..at(old, oc.len() - suf),
        at(new, pre)..at(new, nc.len() - suf),
    ))
}

fn edit_result(b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Row> {
    if o.ok == Some(false) {
        return error_body(&o.failure_text());
    }
    match ToolView::from_details(o.details, o.text) {
        ToolView::Diff { at, old } => {
            let at = at.or_else(|| range_start(b.args));
            let old = old.or_else(|| partial::text(b.args, "old"));
            diff_rows(b.args, at, old)
        }
        ToolView::Plain { text } => match from_args(b.args) {
            Some(rows) => rows,
            None => output_lines(&text),
        },
    }
}

/// A call that hands back no memo — `final`'s edit — draws the change from its own arguments.
fn from_args(args: &str) -> Option<Vec<Row>> {
    let at = range_start(args);
    let old = partial::text(args, "old");
    (at.is_some() || old.is_some()).then(|| diff_rows(args, at, old))
}

/// The first line a `mass` call names.
fn range_start(args: &str) -> Option<usize> {
    let range = partial::text(args, "range")?;
    range.trim().split('-').next()?.trim().parse().ok()
}

/// Removed lines, then added ones, each with the line it sat on.
fn diff_rows(args: &str, at: Option<usize>, old: Option<String>) -> Vec<Row> {
    let new = partial::text(args, "new").unwrap_or_default();
    let deletions = numbered(at, old.as_deref().unwrap_or_default());
    let insertions = numbered(at, &new);
    let width = deletions
        .iter()
        .chain(&insertions)
        .filter_map(|line| line.at)
        .map(|at| at.to_string().len())
        .max()
        .unwrap_or(0);
    let paired = deletions.len() == insertions.len();
    let mut out = Vec::new();
    let mut prev: Option<usize> = None;
    let mut shown: Option<usize> = None;
    for (i, line) in deletions.iter().enumerate() {
        gap(&mut out, prev, line.at);
        prev = line.at;
        let span = paired
            .then(|| {
                changed_spans(
                    &indentation(&line.text).1,
                    &indentation(&insertions[i].text).1,
                )
            })
            .flatten()
            .map(|(removed, _)| removed);
        push(&mut out, '-', line, span, width, &mut shown);
    }
    prev = None;
    for (i, line) in insertions.iter().enumerate() {
        gap(&mut out, prev, line.at);
        prev = line.at;
        let span = paired
            .then(|| {
                changed_spans(
                    &indentation(&deletions[i].text).1,
                    &indentation(&line.text).1,
                )
            })
            .flatten()
            .map(|(_, added)| added);
        push(&mut out, '+', line, span, width, &mut shown);
    }
    if out.is_empty() {
        out.push(Row::text(
            Lead::None,
            "(no changes)",
            Some(Ink::Fixed(fixed::DIFF_CONTEXT)),
        ));
    }
    out
}

/// A text's lines, numbered from `at`.
fn numbered(at: Option<usize>, text: &str) -> Vec<Numbered> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| Numbered {
            at: at.map(|first| first + i),
            text: line.to_string(),
        })
        .collect()
}

/// A row whose number goes when the line above already printed it.
fn push(
    out: &mut Vec<Row>,
    sign: char,
    line: &Numbered,
    span: Option<Range<usize>>,
    width: usize,
    shown: &mut Option<usize>,
) {
    let at = match line.at {
        Some(number) if *shown != Some(number) => {
            *shown = Some(number);
            Some(number)
        }
        _ => None,
    };
    out.push(diff_row(sign, at, &line.text, span, width));
}

/// A jump in line numbers inside one side is a region boundary.
fn gap(out: &mut Vec<Row>, prev: Option<usize>, next: Option<usize>) {
    if let (Some(above), Some(below)) = (prev, next)
        && below > above + 1
    {
        out.push(Row::text(
            Lead::None,
            "…",
            Some(Ink::Fixed(fixed::DIFF_CONTEXT)),
        ));
    }
}
