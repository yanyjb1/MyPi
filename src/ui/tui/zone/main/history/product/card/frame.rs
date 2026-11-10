//! The card itself: name dispatch, the frame primitives, and the shared text helpers.

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::local::content::Block;
use super::super::super::prefetch::color::code;
use super::super::super::row::{Ink, Lead, Row, Run};
use super::super::opts::Opts;
use super::fold;
use super::tool::{ToolBlock, ToolOutcome};
use super::{alias, draft, file, kairos, mcp, partial, task, todo, web};
use crate::ui::tui::text::{bytes, display_width, glyphs as g, take_width};
use crate::ui::tui::theme::fixed;

/// One tool exchange as rows: prose, then the card or the one-line form.
pub fn node(block: &Block, paired: Option<&Block>, opts: &Opts, width: usize) -> Vec<Row> {
    let name = block.name.as_deref().unwrap_or_default();
    let args = block.args.as_deref().unwrap_or_default();
    let details = paired.and_then(details_of);
    let outcome = paired.map(|result| outcome_of(result, details.as_ref()));
    let mut out = Vec::new();

    if name == "todo" {
        out.extend(todo::marker(args, outcome));
        return done(out, block);
    }
    if name == "read" || name == "cd" {
        out.extend(match name {
            "read" => file::read_line(args, outcome, opts.cwd),
            _ => file::cd_line(args, outcome, opts.cwd),
        });
        return done(out, block);
    }
    if name == "alias" {
        out.extend(alias::block(args, outcome));
        return done(out, block);
    }
    if name == "task" {
        out.extend(task::block(args, outcome, opts.cwd));
        return done(out, block);
    }
    if name == "kairos"
        && let Some(rows) = kairos::inline(args, outcome)
    {
        out.extend(rows);
        return done(out, block);
    }
    if name == "mcp"
        && let Some(rows) = mcp::inline(args, outcome)
    {
        out.extend(rows);
        return done(out, block);
    }
    if name == "final"
        && let Some(rows) = draft::inline(args, outcome)
    {
        out.extend(rows);
        return done(out, block);
    }
    if name == "web"
        && let Some(rows) = web::inline(args, outcome)
    {
        out.extend(rows);
        return done(out, block);
    }
    out.extend(card(
        ToolBlock {
            name,
            args,
            intent: block.intent.as_deref(),
            result: outcome,
            expanded: opts.tool,
            budget: opts.fold,
            width,
        },
        outcome.is_none_or(|o| o.ok.is_none()),
        opts.fold,
        width,
    ));
    done(out, block)
}

/// The truncation line: after the family's own rows, outside every fold.
fn done(mut rows: Vec<Row>, block: &Block) -> Vec<Row> {
    if let Some(t) = block.truncated {
        let text = format!(
            "[{} lines & {} truncated, see the #ref later]",
            t.lines,
            bytes(t.bytes)
        );
        rows.push(Row::of(Lead::None, vec![Run::fixed(text, fixed::DIM)]));
    }
    rows
}

/// Header, labeled sections, closing edge.
fn card(block: ToolBlock<'_>, tail: bool, budget: usize, width: usize) -> Vec<Row> {
    let edge = block.edge();
    let bg = block.bg();
    let mut out = vec![card_edge_labeled(edge, bg, width, block.header())];
    let mut sections = block.sections();
    for (index, section) in sections.iter_mut().enumerate() {
        let rows = std::mem::take(&mut section.lines);
        section.lines = fold::fold(rows, budget, block.expanded, tail && index > 0);
    }
    for (i, section) in sections.into_iter().enumerate() {
        if i > 0 || section.label.is_some() {
            out.push(match section.label {
                Some(label) => card_edge_labeled(
                    edge,
                    bg,
                    width,
                    Row::of(Lead::None, vec![Run::fixed(label, fixed::TOOL_TITLE)]),
                ),
                None => card_edge(edge, bg, width),
            });
        }
        for line in section.lines {
            out.push(card_row(line, edge, bg, width));
        }
    }
    out.push(card_edge(edge, bg, width));
    out
}

/// The body of a result that has one, or the placeholder.
pub(crate) fn output_lines(text: &str) -> Vec<Row> {
    if text.trim().is_empty() {
        return vec![Row::of(
            Lead::None,
            vec![Run::fixed("(no output)", fixed::MUTED)],
        )];
    }
    text.lines()
        .map(|line| Row::of(Lead::None, vec![Run::fixed(line, fixed::TOOL_OUTPUT)]))
        .collect()
}

/// The last rows a running result shows: the fold keeps the tail, so only it is built.
pub(crate) fn output_tail(text: &str, budget: usize) -> Vec<Row> {
    if text.trim().is_empty() {
        return vec![Row::of(
            Lead::None,
            vec![Run::fixed("(no output)", fixed::MUTED)],
        )];
    }
    let budget = budget.max(3);
    text.lines()
        .rev()
        .take(budget)
        .collect::<Vec<&str>>()
        .into_iter()
        .rev()
        .map(|line| Row::of(Lead::None, vec![Run::fixed(line, fixed::TOOL_OUTPUT)]))
        .collect()
}

/// The text inside a card when the call failed: every non-empty line, in the error ink.
pub(crate) fn error_body(text: &str) -> Vec<Row> {
    if text.trim().is_empty() {
        return output_lines(text);
    }
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| Row::of(Lead::None, vec![Run::fixed(line, fixed::ERR)]))
        .collect()
}

/// The call as JSON, or as written when it does not parse.
pub(crate) fn default_call(name: &str, args: &str) -> Vec<Row> {
    match call_input(args) {
        Some(v) => {
            let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| args.to_string());
            pretty
                .lines()
                .map(|l| code::line(l, Some("json")))
                .collect()
        }
        None => {
            let lang = language_for_tool(name);
            args.lines().map(|l| code::line(l, lang)).collect()
        }
    }
}

fn language_for_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "bash" => Some("bash"),
        _ => None,
    }
}

/// The path a file tool names, on one line.
pub(crate) fn path_of(b: &ToolBlock<'_>) -> Option<String> {
    partial::text(b.args, "path")
        .filter(|path| !path.trim().is_empty())
        .map(|path| one_line(&path))
}

/// A tab takes three columns; the terminal's own stops would break every gutter.
pub(crate) fn expand_tabs(text: &str) -> String {
    text.replace('\t', "   ")
}

/// The tab's glyph, padded to the columns `expand_tabs` gives it.
const TAB_MARK: &str = " → ";

/// Split off the indentation, drawn as one `·` per space and a three-column arrow per tab.
pub(crate) fn indentation(text: &str) -> (String, String) {
    let lead = text.len() - text.trim_start_matches([' ', '\t']).len();
    let (head, rest) = text.split_at(lead);
    let mut glyphs = String::new();
    for ch in head.chars() {
        match ch {
            '\t' => glyphs.push_str(TAB_MARK),
            _ => glyphs.push_str(g::INDENT),
        }
    }
    (glyphs, expand_tabs(rest))
}

/// The sign and the line number, right-aligned together, then the bar.
pub(crate) fn gutter(sign: char, at: Option<usize>, width: usize) -> String {
    if width == 0 {
        return format!("{sign} ");
    }
    let text = match at {
        Some(number) => format!("{sign}{number}"),
        None => sign.to_string(),
    };
    format!("{text:>pad$}│", pad = width + 1)
}

pub(crate) fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

pub(crate) fn non_empty_lines(text: &str) -> usize {
    non_empty_text_lines(text).len()
}

pub(crate) fn non_empty_text_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

pub(crate) fn short_url(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    truncate(&one_line(rest), 60)
}

pub(crate) fn truncate(text: &str, max: usize) -> String {
    let (cut, w) = take_width(text, max);
    if w < display_width(text) {
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

pub(crate) fn relativize(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if !path.starts_with('/') || base.is_empty() {
        return path.to_string();
    }
    if let Some(rest) = path.strip_prefix(base) {
        if rest.is_empty() {
            return ".".to_string();
        }
        if let Some(rest) = rest.strip_prefix('/') {
            return format!("./{rest}");
        }
    }
    let parent = base
        .rsplit_once('/')
        .map(|(p, _)| if p.is_empty() { "/" } else { p });
    if parent == Some(path) {
        return "..".to_string();
    }
    path.to_string()
}

/// A call's arguments with the envelope taken off: the `input` object.
pub(crate) fn call_input(args: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(args).ok()?;
    v.get("input").cloned()
}

pub(crate) fn arg_bool(args: &str, key: &str) -> Option<bool> {
    call_input(args)?.get(key)?.as_bool()
}

/// A value that may have arrived as a string holding JSON; the server reads that shape too.
pub(crate) fn loose(value: &Value) -> Value {
    match value {
        Value::String(text) => serde_json::from_str(text).unwrap_or_else(|_| value.clone()),
        other => other.clone(),
    }
}

pub(crate) fn arg_str(args: &str, key: &str) -> Option<String> {
    Some(call_input(args)?.get(key)?.as_str()?.to_string())
}

/// One argument as text: strings bare, numbers and flags spelled out.
pub(crate) fn arg_scalar(args: &str, key: &str) -> Option<String> {
    let value = call_input(args)?.get(key)?.clone();
    Some(match value {
        Value::String(text) => text,
        Value::Null => return None,
        other => other.to_string(),
    })
}

/// The memo a tool result carries, decoded.
pub(crate) fn details_of(result: &Block) -> Option<Value> {
    result
        .memo
        .as_deref()
        .and_then(|memo| serde_json::from_str(memo).ok())
}

pub(crate) fn outcome_of<'a>(result: &'a Block, details: Option<&'a Value>) -> ToolOutcome<'a> {
    ToolOutcome {
        text: result.result.as_deref().unwrap_or_default(),
        ok: result.ok,
        details,
        duration_ms: result.duration_ms.max(0) as u64,
    }
}

fn tinted(text: impl Into<String>, ink: Ink) -> Run {
    Run::with(text, Some(ink), Modifier::empty())
}

fn runs_width(runs: &[Run]) -> usize {
    runs.iter().map(Run::width).sum()
}

/// Every run on this card's fill.
fn on_bg(row: Row, bg: Ink) -> Row {
    Row {
        lead: row.lead,
        runs: row.runs.into_iter().map(|run| run.fill(bg)).collect(),
    }
}

/// Longer rows lose their tail.
fn clip_runs(runs: Vec<Run>, max: usize) -> Vec<Run> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for run in runs {
        let w = run.width();
        if used + w <= max {
            used += w;
            out.push(run);
        } else {
            let keep = max.saturating_sub(used);
            if keep > 0 {
                let (cut, _) = take_width(&run.text, keep);
                out.push(Run {
                    text: cut,
                    ink: run.ink,
                    bg: run.bg,
                    attrs: run.attrs,
                });
            }
            break;
        }
    }
    out
}

/// Spaces up to `width`, on the card's fill.
fn pad_to(mut row: Row, width: usize, fill: Ink) -> Row {
    let used = row.width();
    if used < width {
        row.push(Run::plain(" ".repeat(width - used)).fill(fill));
    }
    row
}

fn card_edge(edge: Ink, bg: Ink, width: usize) -> Row {
    if width < 8 {
        let dashes = width.saturating_sub(4);
        return on_bg(
            Row::of(
                Lead::None,
                vec![
                    tinted("+-", edge),
                    tinted("-".repeat(dashes), edge),
                    tinted("-+", edge),
                ],
            ),
            bg,
        );
    }
    let mid = width - 7;
    on_bg(
        Row::of(
            Lead::None,
            vec![
                tinted("+-", edge),
                tinted("- ", edge),
                tinted("-".repeat(mid), edge),
                tinted(" -+", edge),
            ],
        ),
        bg,
    )
}

fn card_edge_labeled(edge: Ink, bg: Ink, width: usize, label: Row) -> Row {
    let budget = width.saturating_sub(7);
    if budget == 0 {
        return card_edge(edge, bg, width);
    }
    let mut shown = if runs_width(&label.runs) > budget {
        clip_runs(label.runs, budget)
    } else {
        label.runs
    };
    let used = runs_width(&shown);
    let mut out = vec![tinted("+- ", edge)];
    out.append(&mut shown);
    if used > 0 {
        out.push(tinted(" ", edge));
    }
    out.push(tinted("-".repeat(budget.saturating_sub(used)), edge));
    out.push(tinted(" -+", edge));
    on_bg(Row::of(Lead::None, out), bg)
}

fn card_row(content: Row, edge: Ink, bg: Ink, width: usize) -> Row {
    let inner = width.saturating_sub(4);
    let mut runs: Vec<Run> = content
        .runs
        .into_iter()
        .map(|mut run| {
            run.text = crate::ansi::strip_ansi(&run.text);
            run
        })
        .collect();
    if runs_width(&runs) > inner {
        runs = clip_runs(runs, inner);
    }
    let mut row = Row::of(Lead::None, vec![tinted("| ", edge)]);
    row.runs.extend(runs);
    let mut row = pad_to(row, width.saturating_sub(2), bg);
    row.push(tinted(" |", edge));
    on_bg(row, bg)
}
