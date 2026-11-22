//! edit and splice: the call as it streams, then the item around the change with its diff.

use std::ops::Range;

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::prefetch::color::code;
use super::super::super::row::{Ink, Lead, Row, Run};
use super::{
    Excerpt, GUTTER_MIN, Numbered, Section, ToolBlock, ToolOutcome, ToolRenderer, ToolView, gutter,
    indentation, output_lines, partial, path_of,
};
use crate::ui::tui::text::glyphs as g;
use crate::ui::tui::theme::fixed;

pub(crate) struct Edit;

impl ToolRenderer for Edit {
    fn path(&self, b: &ToolBlock<'_>) -> Option<String> {
        path_of(b)
    }

    // A splice names the file it writes and nothing else: the change itself is the body.
    fn quiet(&self, b: &ToolBlock<'_>) -> bool {
        b.name == "splice"
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        let mut out = breadcrumb(b);
        if b.result.is_none() {
            out.extend(streaming(b));
        }
        out
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        vec![Section {
            label: None,
            lines: edit_result(b, o),
        }]
    }
}

/// The blocks the changed item sits in: the call's own body, since the result is the diff.
fn breadcrumb(b: &ToolBlock<'_>) -> Vec<Row> {
    let Some(o) = b.result else {
        return Vec::new();
    };
    let ToolView::Diff { scope, .. } = ToolView::from_details(o.details, o.text) else {
        return Vec::new();
    };
    if scope.is_empty() {
        return Vec::new();
    }
    let text = scope.join(&format!(" {} ", g::SEP));
    vec![Row::of(Lead::None, vec![Run::fixed(text, fixed::DIM)])]
}

/// The call as it arrives: the text going, then the text coming. A `splice` call names its ranges
/// as an array, so each one arrives whole and they are told apart by an ellipsis.
fn streaming(b: &ToolBlock<'_>) -> Vec<Row> {
    if b.name == "splice"
        && let rows = splice_streaming(b)
        && !rows.is_empty()
    {
        return rows;
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

/// One `splice` call's ranges as they arrive, each with its own text.
fn splice_streaming(b: &ToolBlock<'_>) -> Vec<Row> {
    let mut out = Vec::new();
    for (at, item) in partial::items(b.args, "edits").iter().enumerate() {
        if at > 0 {
            out.push(Row::text(
                Lead::None,
                "…",
                Some(Ink::Fixed(fixed::DIFF_CONTEXT)),
            ));
        }
        if let Some(text) = item.get("new").and_then(Value::as_str) {
            push_text(&mut out, '+', text, None);
        }
    }
    out
}

// A splice's memo: one segment per range, drawn top to bottom by the line each one names. A segment is the file around the change, then the lines that went and the lines that stand there now; one that named no range carries nought and comes first.
fn segments(b: &ToolBlock<'_>, details: &Value) -> Option<Vec<Row>> {
    let mut list: Vec<&Value> = details.get("segments")?.as_array()?.iter().collect();
    if list.is_empty() {
        return None;
    }
    list.sort_by_key(|seg| number(seg, "fromLine"));
    let width = list
        .iter()
        .flat_map(|seg| [number(seg, "fromLine"), number(seg, "atLine")])
        .chain(
            list.iter()
                .flat_map(|seg| [run(seg, "before"), run(seg, "after")])
                .flatten()
                .map(|run| run.0),
        )
        .max()
        .unwrap_or(0)
        .to_string()
        .len()
        .max(GUTTER_MIN);
    let lang = path_of(b).as_deref().and_then(code::language_for_path);
    let mut painter = code::Painter::new(lang);
    let mut rows: Vec<Row> = Vec::new();
    for (at, seg) in list.into_iter().enumerate() {
        if at > 0 {
            rows.push(seam());
        }
        let mut next = None;
        if let Some((from, lines)) = run(seg, "before") {
            rows.extend(run_rows(&mut painter, from, &lines, width));
            next = Some(from + lines.len());
        }
        let old = seg.get("old").and_then(Value::as_str).unwrap_or_default();
        let new = seg.get("new").and_then(Value::as_str).unwrap_or_default();
        let first = number(seg, "fromLine");
        if first > 0 && next.is_some_and(|after| first > after) {
            rows.push(seam());
        }
        let deletions = numbered(Some(first), old);
        let insertions = numbered(Some(number(seg, "atLine")), new);
        let paired = deletions.len() == insertions.len();
        for (i, line) in deletions.iter().enumerate() {
            let span = paired
                .then(|| {
                    changed_spans(
                        &indentation(&line.text).1,
                        &indentation(&insertions[i].text).1,
                    )
                })
                .flatten()
                .map(|(removed, _)| removed);
            rows.push(diff_row('-', line.at, &line.text, span, width));
        }
        if let Some(error) = seg.get("error").and_then(Value::as_str) {
            rows.push(Row::of(
                Lead::None,
                vec![Run::fixed(error.to_string(), fixed::ERR)],
            ));
        }
        for (i, line) in insertions.iter().enumerate() {
            let span = paired
                .then(|| {
                    changed_spans(
                        &indentation(&deletions[i].text).1,
                        &indentation(&line.text).1,
                    )
                })
                .flatten()
                .map(|(_, added)| added);
            rows.push(diff_row('+', line.at, &line.text, span, width));
        }
        if let Some((from, lines)) = run(seg, "after") {
            let last = number(seg, "atLine") + new.lines().count();
            if from > last {
                rows.push(seam());
            }
            rows.extend(run_rows(&mut painter, from, &lines, width));
        }
    }
    Some(rows)
}

// One side of a segment's excerpt: where it starts, and its lines.
fn run(seg: &Value, key: &str) -> Option<(usize, Vec<String>)> {
    let held = seg.get(key).filter(|v| !v.is_null())?;
    let from = number(held, "from");
    let lines = held
        .get("lines")?
        .as_array()?
        .iter()
        .map(|line| line.as_str().unwrap_or_default().to_string())
        .collect();
    Some((from, lines))
}

// The lines around a change: numbered by their own place, painted by the file's language.
fn run_rows(painter: &mut code::Painter, from: usize, lines: &[String], width: usize) -> Vec<Row> {
    lines
        .iter()
        .enumerate()
        .map(|(at, text)| {
            let mut runs = vec![Run::fixed(gutter(' ', Some(from + at), width), fixed::DIM)];
            for run in painter.runs(text) {
                runs.push(run);
            }
            if runs.len() == 1 {
                runs.push(Run::plain(text.clone()));
            }
            Row::of(Lead::None, runs)
        })
        .collect()
}

/// One number a segment carries; 0 when it carries none.
fn number(seg: &Value, key: &str) -> usize {
    seg.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
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

/// One unchanged line of the item: its old number, and the text painted by its language.
fn context_row(painter: &mut code::Painter, at: usize, text: &str, width: usize) -> Row {
    let mut runs = vec![Run::fixed(
        gutter(' ', Some(at), width),
        fixed::DIFF_CONTEXT,
    )];
    for mut run in painter.runs(text) {
        run.attrs |= Modifier::DIM;
        runs.push(run);
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
    if let Some(rows) = o.details.and_then(|details| segments(b, details)) {
        return rows;
    }
    if o.ok == Some(false) {
        return o
            .text
            .lines()
            .map(|line| Row::text(Lead::None, line, Some(Ink::Fixed(fixed::ERR))))
            .collect();
    }
    match ToolView::from_details(o.details, o.text) {
        ToolView::Diff {
            at, old, excerpt, ..
        } => {
            let at = at.or_else(|| range_start(b.args));
            let old = old.or_else(|| partial::text(b.args, "old"));
            match excerpt {
                Some(excerpt) => item_rows(b, at, old, excerpt),
                None => diff_rows(b.args, at, old),
            }
        }
        ToolView::Plain { text } => match from_args(b.args) {
            Some(rows) => rows,
            None => output_lines(&text),
        },
    }
}

/// The whole item, with only the changed lines signed: everything else is context.
fn item_rows(
    b: &ToolBlock<'_>,
    at: Option<usize>,
    old: Option<String>,
    excerpt: Excerpt,
) -> Vec<Row> {
    let new = partial::text(b.args, "new").unwrap_or_default();
    let before = excerpt.lines.join("\n");
    let after = match b.name {
        "splice" => {
            let at = at.or_else(|| range_start(b.args)).unwrap_or(excerpt.from);
            let start = at.saturating_sub(excerpt.from).min(excerpt.lines.len());
            let removed = old.as_deref().map(|text| text.lines().count()).unwrap_or(0);
            replaced(&excerpt.lines, start, removed, &new)
        }
        _ => match old.as_deref() {
            Some(old) => before.replacen(old, &new, 1),
            None => before.clone(),
        },
    };

    let old_lines: Vec<&str> = before.lines().collect();
    let new_lines: Vec<&str> = after.lines().collect();
    let mut pre = 0;
    while pre < old_lines.len() && pre < new_lines.len() && old_lines[pre] == new_lines[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < old_lines.len().saturating_sub(pre)
        && suf < new_lines.len().saturating_sub(pre)
        && old_lines[old_lines.len() - 1 - suf] == new_lines[new_lines.len() - 1 - suf]
    {
        suf += 1;
    }

    let lang = path_of(b).as_deref().and_then(code::language_for_path);
    let mut painter = code::Painter::new(lang);
    let width = (excerpt.from + old_lines.len()).to_string().len().max(3);
    let mut rows: Vec<Row> = Vec::with_capacity(old_lines.len() + new_lines.len());

    for (i, line) in old_lines[..pre].iter().enumerate() {
        rows.push(context_row(&mut painter, excerpt.from + i, line, width));
    }

    let removed = &old_lines[pre..old_lines.len() - suf];
    let added = &new_lines[pre..new_lines.len() - suf];
    let paired = removed.len() == added.len();
    for (i, line) in removed.iter().enumerate() {
        let span = paired
            .then(|| changed_spans(&indentation(line).1, &indentation(added[i]).1))
            .flatten()
            .map(|(gone, _)| gone);
        rows.push(diff_row(
            '-',
            Some(excerpt.from + pre + i),
            line,
            span,
            width,
        ));
    }
    for (i, line) in added.iter().enumerate() {
        let span = paired
            .then(|| changed_spans(&indentation(&removed[i]).1, &indentation(line).1))
            .flatten()
            .map(|(_, came)| came);
        rows.push(diff_row(
            '+',
            Some(excerpt.from + pre + i),
            line,
            span,
            width,
        ));
    }

    let delta = new_lines.len() as isize - old_lines.len() as isize;
    for (i, line) in old_lines[old_lines.len() - suf..].iter().enumerate() {
        let number = excerpt.from as isize + (old_lines.len() - suf + i) as isize + delta;
        rows.push(context_row(&mut painter, number as usize, line, width));
    }
    if rows.is_empty() {
        rows.push(Row::text(
            Lead::None,
            "(no changes)",
            Some(Ink::Fixed(fixed::DIFF_CONTEXT)),
        ));
    }
    rows
}

/// The item's lines with one region replaced, as `splice` wrote them.
fn replaced(lines: &[String], start: usize, removed: usize, new: &str) -> String {
    let mut out: Vec<&str> = lines[..start].iter().map(String::as_str).collect();
    out.extend(new.lines());
    let end = (start + removed).min(lines.len());
    out.extend(lines[end..].iter().map(String::as_str));
    out.join("\n")
}

/// A call that hands back no memo — `final`'s edit — draws the change from its own arguments.
fn from_args(args: &str) -> Option<Vec<Row>> {
    let at = range_start(args);
    let old = partial::text(args, "old");
    (at.is_some() || old.is_some()).then(|| diff_rows(args, at, old))
}

/// The first line the call names.
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
        out.push(seam());
    }
}

// The seam between two pieces of a file that do not touch.
fn seam() -> Row {
    Row::text(Lead::None, "…", Some(Ink::Fixed(fixed::DIFF_CONTEXT)))
}
