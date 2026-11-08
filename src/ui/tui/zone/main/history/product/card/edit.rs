//! edit: the old and new text as it streams, or the numbered diff of what changed.

use std::ops::Range;

use ratatui::style::Modifier;

use super::super::super::prefetch::color::code;
use super::super::super::row::{Ink, Lead, Row, Run};
use super::{
    Section, ToolBlock, ToolOutcome, ToolRenderer, ToolView, error_body, expand_tabs, output_lines,
    partial, path_meta,
};
use crate::ui::tui::theme::fixed;

pub(crate) struct Edit;

impl ToolRenderer for Edit {
    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        path_meta(b)
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        edit_call(b)
    }

    fn result_sections(&self, _b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        vec![Section {
            label: None,
            lines: edit_result(o),
        }]
    }
}

/// The call as it arrives: `old` and `new`, or one row per line-numbered write.
fn edit_call(b: &ToolBlock<'_>) -> Vec<Row> {
    if b.result.is_some() {
        return Vec::new();
    }
    let path = partial::text(b.args, "path").unwrap_or_default();
    let lang = code::language_for_path(&path);
    let old = partial::text(b.args, "old").filter(|text| !text.is_empty());
    let new = partial::text(b.args, "new").filter(|text| !text.is_empty());
    let changed = match (old.as_deref(), new.as_deref()) {
        (Some(o), Some(n)) if o.lines().count() == 1 && n.lines().count() == 1 => {
            changed_spans(&expand_tabs(o), &expand_tabs(n))
        }
        _ => None,
    };
    let mut out = Vec::new();
    if let Some(old) = old.as_deref() {
        push_lines(
            &mut out,
            '-',
            old,
            lang,
            changed.as_ref().map(|(r, _)| r.clone()),
        );
    }
    if let Some(new) = new.as_deref() {
        push_lines(&mut out, '+', new, lang, changed.map(|(_, r)| r));
    }
    if out.is_empty() {
        for (at, text) in partial::records(b.args, "line", "text") {
            out.push(mass_row(at, &text, lang));
        }
    }
    out
}

/// One row of a by-line edit: the number when it has arrived, the text always.
fn mass_row(at: Option<usize>, text: &str, lang: Option<&str>) -> Row {
    let body = expand_tabs(text);
    let mut runs = match at {
        Some(n) => vec![Run::fixed(format!("+{n}│"), fixed::DIM)],
        None => vec![Run::fixed("+ ", fixed::DIM)],
    };
    let painted = code::line(&body, lang);
    if painted.runs.is_empty() {
        runs.push(Run::plain(body));
    } else {
        runs.extend(painted.runs);
    }
    Row::of(Lead::None, runs)
}

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

fn split_at_bytes(runs: Vec<Run>, lo: usize, hi: usize) -> (Vec<Run>, Vec<Run>, Vec<Run>) {
    let mut before = Vec::new();
    let mut mid = Vec::new();
    let mut after = Vec::new();
    let mut at = 0usize;
    for run in runs {
        let (start, end) = (at, at + run.text.len());
        at = end;
        for (lo, hi, bucket) in [
            (0, lo, &mut before),
            (lo, hi, &mut mid),
            (hi, usize::MAX, &mut after),
        ] {
            let (s, e) = (start.max(lo), end.min(hi));
            if s < e {
                bucket.push(Run {
                    text: run.text[(s - start)..(e - start)].to_string(),
                    ink: run.ink,
                    bg: run.bg,
                    attrs: run.attrs,
                });
            }
        }
    }
    (before, mid, after)
}

/// The call's old or new text, every line signed and the changed run reversed.
fn push_lines(
    out: &mut Vec<Row>,
    sign: char,
    text: &str,
    lang: Option<&str>,
    span: Option<Range<usize>>,
) {
    let ink = if sign == '-' {
        fixed::DIFF_REMOVED
    } else {
        fixed::DIFF_ADDED
    };
    for (i, line) in text.lines().enumerate() {
        let body = expand_tabs(line);
        let mut runs = vec![Run::fixed(format!("{sign} "), ink)];
        let painted = code::line(&body, lang);
        let parts = if painted.runs.is_empty() {
            vec![Run::plain(body.clone())]
        } else {
            painted.runs
        };
        let span = if i == 0 { span.clone() } else { None };
        match span {
            Some(r) if r.start < r.end && r.end <= body.len() => {
                let (pre, mid, post) = split_at_bytes(parts, r.start, r.end);
                runs.extend(pre);
                for mut run in mid {
                    run.attrs |= Modifier::REVERSED;
                    runs.push(run);
                }
                runs.extend(post);
            }
            _ => runs.extend(parts),
        }
        out.push(Row::of(Lead::None, runs));
    }
}

fn edit_result(o: ToolOutcome<'_>) -> Vec<Row> {
    if o.ok == Some(false) {
        return error_body(o.text);
    }
    match ToolView::from_details(o.details, o.text) {
        ToolView::Diff {
            deletions,
            insertions,
        } => diff_rows(&deletions, &insertions),
        ToolView::Plain { text } => output_lines(&text),
    }
}

/// Removed lines, then added ones, each with the line it sat on.
fn diff_rows(deletions: &[super::Numbered], insertions: &[super::Numbered]) -> Vec<Row> {
    let width = deletions
        .iter()
        .chain(insertions)
        .filter_map(|line| line.at)
        .map(|at| at.to_string().len())
        .max()
        .unwrap_or(0);
    let paired = deletions.len() == insertions.len();
    let mut out = Vec::new();
    let mut prev: Option<usize> = None;
    for (i, line) in deletions.iter().enumerate() {
        gap(&mut out, prev, line.at);
        prev = line.at;
        let span = paired
            .then(|| {
                changed_spans(&expand_tabs(&line.text), &expand_tabs(&insertions[i].text))
                    .map(|(r, _)| r)
            })
            .flatten();
        push_row(&mut out, '-', line, span, width);
    }
    prev = None;
    for (i, line) in insertions.iter().enumerate() {
        gap(&mut out, prev, line.at);
        prev = line.at;
        let span = paired
            .then(|| {
                changed_spans(&expand_tabs(&deletions[i].text), &expand_tabs(&line.text))
                    .map(|(_, r)| r)
            })
            .flatten();
        push_row(&mut out, '+', line, span, width);
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

fn push_row(
    out: &mut Vec<Row>,
    sign: char,
    line: &super::Numbered,
    span: Option<Range<usize>>,
    width: usize,
) {
    let ink = if sign == '-' {
        fixed::DIFF_REMOVED
    } else {
        fixed::DIFF_ADDED
    };
    let mut runs = Vec::new();
    if width > 0 {
        match line.at {
            Some(at) => runs.push(Run::fixed(format!("{sign}{at:>width$}│"), fixed::DIM)),
            None => runs.push(Run::fixed(format!("{sign} "), fixed::DIM)),
        }
    } else {
        runs.push(Run::fixed(format!("{sign} "), fixed::DIM));
    }
    let body = expand_tabs(&line.text);
    match span {
        Some(r) if r.start < r.end && r.end <= body.len() => {
            runs.push(Run::with(
                body[..r.start].to_string(),
                Some(Ink::Fixed(ink)),
                Modifier::empty(),
            ));
            runs.push(Run::with(
                body[r.start..r.end].to_string(),
                Some(Ink::Fixed(ink)),
                Modifier::REVERSED,
            ));
            runs.push(Run::with(
                body[r.end..].to_string(),
                Some(Ink::Fixed(ink)),
                Modifier::empty(),
            ));
        }
        _ => runs.push(Run::with(body, Some(Ink::Fixed(ink)), Modifier::empty())),
    }
    out.push(Row::of(Lead::None, runs));
}
