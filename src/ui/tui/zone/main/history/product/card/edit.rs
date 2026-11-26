// edit and splice: the call as it streams, then the item around the change with its diff.

use std::ops::Range;

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::prefetch::color::code;
use super::super::super::row::{Ink, Lead, Row, Run};
use super::{
    Excerpt, GUTTER_MIN, Numbered, Section, ToolBlock, ToolOutcome, ToolRenderer, ToolView, gutter,
    indentation, output_lines, partial, path_of,
};
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
        if b.result.is_none() {
            return streaming(b);
        }
        Vec::new()
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        vec![Section {
            label: None,
            lines: edit_result(b, o),
        }]
    }
}

// The call as it arrives: the text going, then the text coming; a `splice` call's ranges are told apart by an ellipsis.
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

// One `splice` call's ranges as they arrive, each with its own text.
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
                .flat_map(|seg| {
                    [
                        run(seg, "before"),
                        run(seg, "after"),
                        Some(change(seg, "old")),
                        Some(change(seg, "new")),
                    ]
                })
                .flat_map(Option::unwrap_or_default)
                .map(|line| line.at.unwrap_or(0)),
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
        if let Some(lines) = run(seg, "before") {
            next = lines.last().and_then(|line| line.at).map(|at| at + 1);
            rows.extend(context_rows(&mut painter, &lines, width));
        }
        let deletions = change(seg, "old");
        let insertions = change(seg, "new");
        let first = number(seg, "fromLine");
        if first > 0 && next.is_some_and(|after| first > after) {
            rows.push(seam());
        }
        let both = deletions.len() == insertions.len();
        for (gone, lines) in [('-', &deletions), ('+', &insertions)] {
            for (i, line) in lines.iter().enumerate() {
                let span = both
                    .then(|| {
                        changed_spans(
                            &indentation(&deletions[i].text).1,
                            &indentation(&insertions[i].text).1,
                        )
                    })
                    .flatten()
                    .map(|(removed, added)| if gone == '-' { removed } else { added });
                rows.push(diff_row(gone, line.at, &line.text, span, width));
            }
            if gone == '-'
                && let Some(error) = seg.get("error").and_then(Value::as_str)
            {
                rows.push(Row::of(
                    Lead::None,
                    vec![Run::fixed(error.to_string(), fixed::ERR)],
                ));
            }
        }
        if let Some(lines) = run(seg, "after") {
            let last = insertions
                .last()
                .map_or(number(seg, "atLine"), |line| line.at.unwrap_or(0));
            if lines
                .first()
                .is_some_and(|line| line.at.is_some_and(|at| at > last))
            {
                rows.push(seam());
            }
            rows.extend(context_rows(&mut painter, &lines, width));
        }
    }
    Some(rows)
}

// One side of the file around a segment, each line with the place it stands on; `None` when the segment carries no such side.
fn run(seg: &Value, key: &str) -> Option<Vec<Numbered>> {
    let held = seg.get(key).filter(|v| !v.is_null())?;
    paired(held.get("lines")?.as_array()?)
}

// One side of the change itself, each line with the place it stands on.
fn change(seg: &Value, key: &str) -> Vec<Numbered> {
    seg.get(key)
        .and_then(Value::as_array)
        .and_then(|list| paired(list))
        .unwrap_or_default()
}

// Lines as the memo carries them: `[place, text]` pairs, in file order.
fn paired(list: &[Value]) -> Option<Vec<Numbered>> {
    list.iter()
        .map(|pair| {
            let pair = pair.as_array()?;
            Some(Numbered {
                at: usize::try_from(pair.first()?.as_u64()?).ok(),
                text: pair.get(1)?.as_str()?.to_string(),
            })
        })
        .collect()
}

// The lines around a change: numbered by their own place, painted by the file's language.
fn context_rows(painter: &mut code::Painter, lines: &[Numbered], width: usize) -> Vec<Row> {
    lines
        .iter()
        .map(|line| {
            let mut runs = vec![Run::fixed(gutter(' ', line.at, width), fixed::DIM)];
            for run in painter.runs(&line.text) {
                runs.push(run);
            }
            if runs.len() == 1 {
                runs.push(Run::plain(line.text.clone()));
            }
            Row::of(Lead::None, runs)
        })
        .collect()
}

// One number a segment carries; 0 when it carries none.
fn number(seg: &Value, key: &str) -> usize {
    seg.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
}

// Text that arrived whole: every line signed, without a number.
fn push_text(out: &mut Vec<Row>, sign: char, text: &str, span: Option<Range<usize>>) {
    for (i, line) in text.lines().enumerate() {
        let span = if i == 0 { span.clone() } else { None };
        out.push(diff_row(sign, None, line, span, 0));
    }
}

// One diff row: the sign and the number, the indentation, then the text.
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

// One unchanged line of the item: its old number, and the text painted by its language.
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

// The two single lines' changed runs; the indentation is never part of them.
fn pair_spans(old: &str, new: &str) -> Option<(Range<usize>, Range<usize>)> {
    if old.lines().count() != 1 || new.lines().count() != 1 {
        return None;
    }
    let old = indentation(old.lines().next().unwrap_or_default()).1;
    let new = indentation(new.lines().next().unwrap_or_default()).1;
    changed_spans(&old, &new)
}

// The run one line has before and after its common prefix and suffix.
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

// The whole item, with only the changed lines signed: everything else is context. Every line carries its own file line, because a line the mask folded stands for a run and no longer stands beside its neighbour.
fn item_rows(
    b: &ToolBlock<'_>,
    at: Option<usize>,
    old: Option<String>,
    excerpt: Excerpt,
) -> Vec<Row> {
    let new = partial::text(b.args, "new").unwrap_or_default();
    let before: Vec<(usize, String)> = excerpt
        .lines
        .iter()
        .cloned()
        .zip(excerpt.at.iter().copied())
        .map(|(text, at)| (at, text))
        .collect();
    let flat: String = before
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let after = match b.name {
        "splice" => {
            let at = at
                .or_else(|| range_start(b.args))
                .unwrap_or(excerpt.at.first().copied().unwrap_or(1));
            let start = before
                .iter()
                .position(|(line, _)| *line >= at)
                .unwrap_or(before.len());
            let removed = old.as_deref().map(|text| text.lines().count()).unwrap_or(0);
            let texts: Vec<String> = before.iter().map(|(_, text)| text.clone()).collect();
            let replaced_text = replaced(&texts, start, removed, &new);
            merged(&before, start, removed, replaced_text)
        }
        _ => match old.as_deref() {
            Some(old) => match flat.replacen(old, &new, 1) {
                text if text != flat => renumber(&before, &text),
                _ => before.clone(),
            },
            None => before.clone(),
        },
    };

    let mut pre = 0;
    while pre < before.len() && pre < after.len() && before[pre].1 == after[pre].1 {
        pre += 1;
    }
    let mut suf = 0;
    while suf < before.len().saturating_sub(pre)
        && suf < after.len().saturating_sub(pre)
        && before[before.len() - 1 - suf].1 == after[after.len() - 1 - suf].1
    {
        suf += 1;
    }

    let lang = path_of(b).as_deref().and_then(code::language_for_path);
    let mut painter = code::Painter::new(lang);
    let width = before
        .iter()
        .chain(after.iter())
        .map(|(at, _)| *at)
        .max()
        .unwrap_or(0)
        .to_string()
        .len()
        .max(3);
    let mut rows: Vec<Row> = Vec::with_capacity(before.len() + after.len());

    for (at, line) in &before[..pre] {
        rows.push(context_row(&mut painter, *at, line, width));
    }

    let removed = &before[pre..before.len() - suf];
    let added = &after[pre..after.len() - suf];
    let paired = removed.len() == added.len();
    for (i, (at, line)) in removed.iter().enumerate() {
        let span = paired
            .then(|| changed_spans(&indentation(line).1, &indentation(&added[i].1).1))
            .flatten()
            .map(|(gone, _)| gone);
        rows.push(diff_row('-', Some(*at), line, span, width));
    }
    let delta = after.len() as isize - before.len() as isize;
    for (i, (at, line)) in added.iter().enumerate() {
        let span = paired
            .then(|| changed_spans(&indentation(&removed[i].1).1, &indentation(line).1))
            .flatten()
            .map(|(_, came)| came);
        rows.push(diff_row('+', Some(*at), line, span, width));
    }

    for (at, line) in &before[before.len() - suf..] {
        let number = (*at as isize + delta).max(1) as usize;
        rows.push(context_row(&mut painter, number, line, width));
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

// The item after a block was cut out of it: the lines above keep their place, the block that came takes the first removed line's place, and the lines below move by as many as were added.
fn merged(
    before: &[(usize, String)],
    start: usize,
    removed: usize,
    text: String,
) -> Vec<(usize, String)> {
    let opens = before.get(start).map_or(1, |(at, _)| *at);
    let mut out: Vec<(usize, String)> = before[..start].to_vec();
    for (back, line) in text.lines().enumerate() {
        out.push((opens + back, line.to_string()));
    }
    let below = start + removed;
    let shift = text.lines().count() as isize - removed as isize;
    for (at, line) in &before[below.min(before.len())..] {
        let moved = (*at as isize + shift).max(1) as usize;
        out.push((moved, line.clone()));
    }
    out
}

// The item after one run of text was swapped for another, its lines keeping the place the walk gives them.
fn renumber(before: &[(usize, String)], text: &str) -> Vec<(usize, String)> {
    let opens = before.first().map_or(1, |(at, _)| *at);
    text.lines()
        .enumerate()
        .map(|(back, line)| (opens + back, line.to_string()))
        .collect()
}

fn replaced(lines: &[String], start: usize, removed: usize, new: &str) -> String {
    let mut out: Vec<&str> = lines[..start].iter().map(String::as_str).collect();
    out.extend(new.lines());
    let end = (start + removed).min(lines.len());
    out.extend(lines[end..].iter().map(String::as_str));
    out.join("\n")
}

// A call that hands back no memo — `final`'s edit — draws the change from its own arguments.
fn from_args(args: &str) -> Option<Vec<Row>> {
    let at = range_start(args);
    let old = partial::text(args, "old");
    (at.is_some() || old.is_some()).then(|| diff_rows(args, at, old))
}

// The first line the call names.
fn range_start(args: &str) -> Option<usize> {
    let range = partial::text(args, "range")?;
    range.trim().split('-').next()?.trim().parse().ok()
}

// Removed lines, then added ones, each with the line it sat on.
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
// A text's lines, numbered from `at`.
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

// A row whose number goes when the line above already printed it.
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

// A jump in line numbers inside one side is a region boundary.
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
