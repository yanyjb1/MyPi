//! edit: the old and new text, or the result as a numbered diff.

use std::ops::Range;

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::prefetch::color::code;
use super::super::super::row::{Ink, Lead, Row, Run};
use super::{
    Section, ToolBlock, ToolOutcome, ToolRenderer, ToolView, arg_str, output_lines, path_meta,
};
use crate::ui::tui::theme::{contrast_text_on, fixed};

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

fn edit_call(b: &ToolBlock<'_>) -> Vec<Row> {
    if b.result.is_some() {
        return Vec::new();
    }
    let path = arg_str(b.args, "path").unwrap_or_default();
    let lang = code::language_for_path(&path);
    let old_text = arg_str(b.args, "old");
    let new_text = arg_str(b.args, "new");
    let changed = match (old_text.as_deref(), new_text.as_deref()) {
        (Some(o), Some(n)) if o.lines().count() == 1 && n.lines().count() == 1 => {
            changed_spans(o, n)
        }
        _ => None,
    };
    let mut out = Vec::new();
    if let Some(old) = old_text {
        push_diff(
            &mut out,
            '-',
            &old,
            changed.as_ref().map(|(r, _)| r.clone()),
            lang,
        );
    }
    if let Some(new) = new_text {
        push_diff(&mut out, '+', &new, changed.map(|(_, r)| r), lang);
    }
    if out.is_empty()
        && let Some(edits) = serde_json::from_str::<Value>(b.args)
            .ok()
            .and_then(|v| v.get("edits").and_then(|e| e.as_array()).cloned())
    {
        let mut text = String::new();
        for e in edits {
            let line = e.get("line").and_then(|l| l.as_i64()).unwrap_or(0);
            let body = e.get("text").and_then(|l| l.as_str()).unwrap_or("");
            text.push_str(&format!("{line}: {body}\n"));
        }
        push_diff(&mut out, '+', text.trim_end(), None, lang);
    }
    out
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

fn push_diff(
    out: &mut Vec<Row>,
    sign: char,
    text: &str,
    changed: Option<Range<usize>>,
    lang: Option<&str>,
) {
    let color = if sign == '-' {
        fixed::DIFF_REMOVED
    } else {
        fixed::DIFF_ADDED
    };
    for (i, l) in text.lines().enumerate() {
        let mut runs = vec![Run::fixed(format!("{sign} "), color)];
        let row = code::line(l, lang);
        let hl = if row.runs.is_empty() {
            None
        } else {
            Some(row.runs)
        };
        let r = if i == 0 { changed.clone() } else { None };
        match (hl, r) {
            (Some(parts), Some(r)) => {
                let (pre, mid, post) = split_at_bytes(parts, r.start, r.end);
                runs.extend(pre);
                for mut run in mid {
                    run.attrs |= Modifier::REVERSED;
                    runs.push(run);
                }
                runs.extend(post);
            }
            (Some(parts), None) => runs.extend(parts),
            (None, _) => runs.push(Run::plain(l)),
        }
        out.push(Row::of(Lead::None, runs));
    }
}

fn edit_result(o: ToolOutcome<'_>) -> Vec<Row> {
    match ToolView::from_details(o.details, o.text) {
        ToolView::Diff {
            deletions,
            insertions,
            at_line,
        } => {
            let fg = Ink::Fixed(contrast_text_on(fixed::TOOL_BG_OK));
            let numbers = at_line.map(|at| {
                let last = at + deletions.len().max(insertions.len()).saturating_sub(1);
                last.to_string().len()
            });
            let gutter = |sign: char, idx: usize, blank_number: bool| -> String {
                match (at_line, numbers) {
                    (Some(at), Some(w)) => {
                        let text = if blank_number {
                            sign.to_string()
                        } else {
                            format!("{sign}{}", at + idx)
                        };
                        format!("{text:>pad$}│", pad = w + 1)
                    }
                    _ => format!("{sign} "),
                }
            };
            let mut out = Vec::new();
            let changed = (deletions.len() == 1 && insertions.len() == 1)
                .then(|| changed_spans(&deletions[0], &insertions[0]))
                .flatten();
            let mut last_shown: Option<usize> = None;
            let mut emit = |sign: char, idx: usize, text: &str, r: Option<Range<usize>>| {
                let n = at_line.map(|at| at + idx);
                let blank = matches!((n, last_shown), (Some(a), Some(b)) if a == b);
                last_shown = n;
                let lead = gutter(sign, idx, blank);
                let mut runs = Vec::new();
                match r {
                    Some(r) => {
                        runs.push(Run::with(
                            format!("{lead}{}", &text[..r.start]),
                            Some(fg),
                            Modifier::empty(),
                        ));
                        runs.push(Run::with(
                            text[r.start..r.end].to_string(),
                            Some(fg),
                            Modifier::REVERSED,
                        ));
                        runs.push(Run::with(
                            text[r.end..].to_string(),
                            Some(fg),
                            Modifier::empty(),
                        ));
                    }
                    None => runs.push(Run::with(
                        format!("{lead}{text}"),
                        Some(fg),
                        Modifier::empty(),
                    )),
                }
                out.push(Row::of(Lead::None, runs));
            };
            for (i, d) in deletions.iter().enumerate() {
                emit('-', i, d, changed.as_ref().map(|(r, _)| r.clone()));
            }
            for (i, ins) in insertions.iter().enumerate() {
                emit('+', i, ins, changed.as_ref().map(|(_, r)| r.clone()));
            }
            out
        }
        ToolView::Plain { text } => output_lines(&text),
    }
}
