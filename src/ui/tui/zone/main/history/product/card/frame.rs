//! The card itself: the renderer interface, name dispatch, the frame primitives.

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::local::content::Block;
use super::super::super::prefetch::color::code;
use super::super::super::row::{Ink, Lead, Row, Run};
use super::super::opts::Opts;
use super::fold;
use super::{
    alias, bash, context, draft, edit, fallback, file, kairos, mcp, memo, partial, task, todo, web,
};
use crate::ui::tui::text::{display_width, glyphs as g, take_width};
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
        return out;
    }
    if name == "read" || name == "cd" {
        out.extend(match name {
            "read" => file::read_line(args, outcome, opts.cwd),
            _ => file::cd_line(args, outcome, opts.cwd),
        });
        return out;
    }
    if name == "alias" {
        out.extend(alias::block(args, outcome));
        return out;
    }
    if name == "task" {
        out.extend(task::block(args, outcome, opts.cwd));
        return out;
    }
    if name == "kairos"
        && let Some(rows) = kairos::inline(args, outcome)
    {
        out.extend(rows);
        return out;
    }
    if name == "mcp"
        && let Some(rows) = mcp::inline(args, outcome)
    {
        out.extend(rows);
        return out;
    }
    if name == "final"
        && let Some(rows) = draft::inline(args, outcome)
    {
        out.extend(rows);
        return out;
    }
    if name == "web"
        && let Some(rows) = web::inline(args, outcome)
    {
        out.extend(rows);
        return out;
    }
    out.extend(card(
        ToolBlock {
            name,
            args,
            result: outcome,
            expanded: opts.tool,
            width,
        },
        outcome.is_none_or(|o| o.ok.is_none()),
        opts.fold,
        width,
    ));
    out
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

/// What a paired result says about the call.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolOutcome<'a> {
    pub text: &'a str,
    /// `None` while the call is still running.
    pub ok: Option<bool>,
    pub details: Option<&'a Value>,
    pub duration_ms: u64,
}

impl ToolOutcome<'_> {
    fn duration_label(&self) -> Option<String> {
        (self.duration_ms >= 1_000).then(|| format_duration(self.duration_ms))
    }
}

/// One tool exchange as the renderers see it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolBlock<'a> {
    pub name: &'a str,
    pub args: &'a str,
    pub result: Option<ToolOutcome<'a>>,
    pub expanded: bool,
    pub width: usize,
}

impl ToolBlock<'_> {
    /// The columns a row may take inside the frame.
    pub(crate) fn inner_width(&self) -> usize {
        self.width.saturating_sub(4)
    }

    fn renderer(&self) -> &'static dyn ToolRenderer {
        renderer_for(self.name)
    }

    fn state(&self) -> State {
        State::of(self.result)
    }

    fn edge(&self) -> Ink {
        self.state().edge()
    }

    fn bg(&self) -> Ink {
        self.state().bg()
    }

    fn header(&self) -> Row {
        let state = self.state();
        let shown = self
            .renderer()
            .label(self)
            .unwrap_or_else(|| self.name.to_string());
        let mut runs = vec![
            Run::with(state.glyph(), Some(state.glyph_ink()), Modifier::empty()),
            Run::fixed(format!(" {shown}"), fixed::TOOL_TITLE).bold(),
        ];
        if let Some(title) = self.renderer().title(self) {
            runs.push(Run::fixed(format!(": {title}"), fixed::MUTED));
            return Row::of(Lead::None, runs);
        }
        if let Some(d) = description(self.args) {
            runs.push(Run::fixed(format!(": {d}"), fixed::MUTED));
        }
        if let Some(path) = self.renderer().path(self) {
            runs.push(Run::fixed(format!(" {} ", g::SEP), fixed::DIM));
            runs.push(Run::fixed(path, fixed::TOOL_PATH));
        }
        for m in self.meta() {
            runs.push(Run::fixed(format!(" {} {m}", g::SEP), fixed::DIM));
        }
        Row::of(Lead::None, runs)
    }

    fn meta(&self) -> Vec<String> {
        let r = self.renderer();
        let mut out = r.meta(self);
        if let Some(o) = self.result {
            if let Some(m) = r.result_measure(o) {
                out.push(m);
            }
            if let Some(d) = o.duration_label() {
                out.push(d);
            }
        }
        out
    }

    fn call_body(&self) -> Vec<Row> {
        self.renderer().call_body(self)
    }

    fn result_sections(&self, o: ToolOutcome<'_>) -> Vec<Section> {
        self.renderer().result_sections(self, o)
    }

    fn sections(&self) -> Vec<Section> {
        let mut out = Vec::new();
        let call = self.call_body();
        if !call.is_empty() {
            out.push(Section {
                label: None,
                lines: call,
            });
        }
        if let Some(o) = self.result {
            out.extend(self.result_sections(o));
        }
        out.retain(|s| !s.lines.is_empty());
        out
    }
}

/// One run of what a card shows, under a label it may carry.
pub(crate) struct Section {
    pub label: Option<&'static str>,
    pub lines: Vec<Row>,
}

/// How a card is colored, by the state of the exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// No settled result yet: the call is queued, its arguments still arrive, or its output streams.
    Pending,
    Ok,
    Err,
}

impl State {
    pub(crate) fn of(result: Option<ToolOutcome<'_>>) -> State {
        match result.and_then(|o| o.ok) {
            None => State::Pending,
            Some(true) => State::Ok,
            Some(false) => State::Err,
        }
    }

    pub(crate) fn glyph(self) -> &'static str {
        match self {
            State::Pending => g::PENDING,
            State::Ok => g::OK,
            State::Err => g::ERR,
        }
    }

    pub(crate) fn glyph_ink(self) -> Ink {
        match self {
            State::Pending => Ink::Accent,
            State::Ok => Ink::Fixed(fixed::OK),
            State::Err => Ink::Fixed(fixed::ERR),
        }
    }

    fn edge(self) -> Ink {
        match self {
            State::Pending => Ink::Accent,
            State::Ok => Ink::Fixed(fixed::TOOL_EDGE_OK),
            State::Err => Ink::Fixed(fixed::TOOL_EDGE_ERR),
        }
    }

    fn bg(self) -> Ink {
        Ink::Fixed(match self {
            State::Pending => fixed::TOOL_BG_PENDING,
            State::Ok => fixed::TOOL_BG_OK,
            State::Err => fixed::TOOL_BG_ERR,
        })
    }
}

/// What one tool family draws for the call and for the result.
pub(crate) trait ToolRenderer {
    /// The name shown in the header, when it is not the tool's own.
    fn label(&self, _b: &ToolBlock<'_>) -> Option<String> {
        None
    }

    /// The text after the header's colon, when the card names it itself.
    fn title(&self, _b: &ToolBlock<'_>) -> Option<String> {
        None
    }

    fn meta(&self, _b: &ToolBlock<'_>) -> Vec<String> {
        Vec::new()
    }

    /// The file this call names, shown in the path color.
    fn path(&self, _b: &ToolBlock<'_>) -> Option<String> {
        None
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        let lines = non_empty_lines(o.text);
        (lines > 1).then(|| format!("{lines} lines"))
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        default_call(b.name, b.args)
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        let lines = match o.ok {
            Some(false) => error_body(o.text),
            _ if b.name == "read" => file::read_body(o),
            _ => output_lines(o.text),
        };
        vec![Section { label: None, lines }]
    }
}

fn renderer_for(name: &str) -> &'static dyn ToolRenderer {
    match name {
        "bash" => &bash::Bash,
        "edit" => &edit::Edit,
        "read" | "write" | "cd" => &file::FileTools,
        "web" => &web::Web,
        "context" => &context::Context,
        "memo" => &memo::Memo,
        "mcp" => &mcp::Call,
        "kairos" => &kairos::Payload,
        "final" => &draft::Draft,
        _ => &fallback::Fallback,
    }
}

/// The status glyph and a bold title, for the one-line forms.
pub(crate) fn head(state: State, label: &str) -> Vec<Run> {
    vec![
        Run::with(state.glyph(), Some(state.glyph_ink()), Modifier::empty()),
        Run::fixed(format!(" {label}"), fixed::TOOL_TITLE).bold(),
    ]
}

/// The failure text under a one-line form: the card body, shifted under the title.
pub(crate) fn error_under(result: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let Some(o) = result.filter(|o| o.ok == Some(false)) else {
        return Vec::new();
    };
    error_body(o.text)
        .into_iter()
        .map(|mut row| {
            if let Some(first) = row.runs.first_mut() {
                first.text.insert_str(0, "  ");
            }
            row
        })
        .collect()
}

/// One diff line: the line it sat on, and its text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Numbered {
    pub at: Option<usize>,
    pub text: String,
}

/// The result text a diff-carrying edit replaces; a plain line otherwise.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ToolView {
    Plain {
        text: String,
    },
    Diff {
        deletions: Vec<Numbered>,
        insertions: Vec<Numbered>,
    },
}

impl ToolView {
    pub(crate) fn from_details(details: Option<&Value>, text: &str) -> ToolView {
        let diff = details.filter(|d| d.get("kind").and_then(|k| k.as_str()) == Some("diff"));
        if let Some(d) = diff {
            let lines = |key: &str| -> Vec<Numbered> {
                let mut out: Vec<Numbered> = d
                    .get(key)
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                Some(Numbered {
                                    at: item
                                        .get("line")
                                        .and_then(Value::as_u64)
                                        .map(|n| n as usize),
                                    text: item.get("text").and_then(Value::as_str)?.to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out.sort_by_key(|line| line.at.unwrap_or(usize::MAX));
                out
            };
            let (deletions, insertions) = (lines("deletions"), lines("insertions"));
            if !deletions.is_empty() || !insertions.is_empty() {
                return ToolView::Diff {
                    deletions,
                    insertions,
                };
            }
        }
        ToolView::Plain {
            text: text.to_string(),
        }
    }
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

/// The failure text inside a card: every line in the error color.
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
    match serde_json::from_str::<Value>(args) {
        Ok(v) => {
            let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| args.to_string());
            pretty
                .lines()
                .map(|l| code::line(l, Some("json")))
                .collect()
        }
        Err(_) => {
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

fn description(args: &str) -> Option<String> {
    let v: Value = serde_json::from_str(args).ok()?;
    let intent = v.get("intent")?.as_str()?.trim();
    if intent.is_empty() {
        return None;
    }
    Some(one_line(intent))
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

pub(crate) fn arg_bool(args: &str, key: &str) -> Option<bool> {
    let v: Value = serde_json::from_str(args).ok()?;
    v.get(key)?.as_bool()
}

pub(crate) fn arg_str(args: &str, key: &str) -> Option<String> {
    let v: Value = serde_json::from_str(args).ok()?;
    Some(v.get(key)?.as_str()?.to_string())
}

/// One argument as text: strings bare, numbers and flags spelled out.
pub(crate) fn arg_scalar(args: &str, key: &str) -> Option<String> {
    let v: Value = serde_json::from_str(args).ok()?;
    let value = v.get(key)?;
    Some(match value {
        Value::String(text) => text.clone(),
        Value::Null => return None,
        other => other.to_string(),
    })
}

fn format_duration(ms: u64) -> String {
    const SEC: u64 = 1_000;
    const MIN: u64 = 60 * SEC;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    if ms < SEC {
        return format!("{ms}ms");
    }
    if ms < MIN {
        return format!("{:.1}s", ms as f64 / SEC as f64);
    }
    if ms < HOUR {
        let (m, s) = (ms / MIN, (ms % MIN) / SEC);
        return if s > 0 {
            format!("{m}m{s}s")
        } else {
            format!("{m}m")
        };
    }
    if ms < DAY {
        let (h, m) = (ms / HOUR, (ms % HOUR) / MIN);
        return if m > 0 {
            format!("{h}h{m}m")
        } else {
            format!("{h}h")
        };
    }
    let (d, h) = (ms / DAY, (ms % DAY) / HOUR);
    if h > 0 {
        format!("{d}d{h}h")
    } else {
        format!("{d}d")
    }
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
