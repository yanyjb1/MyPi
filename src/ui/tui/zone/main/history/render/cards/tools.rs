//! Tool cards: a header, labeled sections, and the result body, by tool name.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::super::text::glyphs as g;
use super::super::text::highlight;
use super::super::theme::{HistoryTheme, Token};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ToolView {
    Plain {
        text: String,
    },
    Diff {
        deletions: Vec<String>,
        insertions: Vec<String>,
        at_line: Option<usize>,
    },
}

impl ToolView {
    pub(super) fn from_details(details: Option<&serde_json::Value>, text: &str) -> ToolView {
        let diff = details.filter(|d| d.get("kind").and_then(|k| k.as_str()) == Some("diff"));
        if let Some(d) = diff {
            let lines = |key: &str| -> Vec<String> {
                d.get(key)
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str())
                            .map(String::from)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            let (deletions, insertions) = (lines("deletions"), lines("insertions"));
            let at_line = d.get("atLine").and_then(|v| v.as_u64()).map(|n| n as usize);
            if !deletions.is_empty() || !insertions.is_empty() {
                return ToolView::Diff {
                    deletions,
                    insertions,
                    at_line,
                };
            }
        }
        ToolView::Plain {
            text: text.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ToolOutcome<'a> {
    pub text: &'a str,
    pub ok: bool,
    pub details: Option<&'a serde_json::Value>,
    pub duration_ms: u64,
}

impl<'a> ToolOutcome<'a> {
    pub(super) fn duration_label(&self) -> Option<String> {
        (self.duration_ms >= 1_000).then(|| format_duration(self.duration_ms))
    }
}

pub(super) fn format_duration(ms: u64) -> String {
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

const DEFAULT_FOLD: usize = 5;
const DIFF_FOLD: usize = 14;
const WEB_FOLD: usize = 3;
const WEB_OPEN: usize = 12;

pub(super) struct Section {
    pub label: Option<&'static str>,
    pub lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    Pending,
    Ok,
    Err,
}

impl State {
    fn glyph(self) -> &'static str {
        match self {
            State::Pending => g::PENDING,
            State::Ok => g::OK,
            State::Err => g::ERR,
        }
    }

    fn glyph_token(self) -> Token {
        match self {
            State::Pending => Token::ToolEdgePending,
            State::Ok => Token::Ok,
            State::Err => Token::Err,
        }
    }

    fn edge_token(self) -> Token {
        match self {
            State::Pending => Token::ToolEdgePending,
            State::Ok => Token::ToolEdgeSuccess,
            State::Err => Token::ToolEdgeError,
        }
    }

    fn bg_token(self) -> Token {
        match self {
            State::Pending => Token::ToolBgPending,
            State::Ok => Token::ToolBgSuccess,
            State::Err => Token::ToolBgError,
        }
    }
}

pub(super) trait ToolRenderer {
    fn fold_limit(&self) -> usize {
        DEFAULT_FOLD
    }

    fn fold_takes_head(&self) -> bool {
        false
    }

    fn meta(&self, _b: &ToolBlock<'_>) -> Vec<String> {
        Vec::new()
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        let lines = non_empty_lines(o.text);
        (lines > 1).then(|| format!("{lines} lines"))
    }

    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        default_call(b.name, b.args, t)
    }

    fn result_sections(
        &self,
        b: &ToolBlock<'_>,
        o: ToolOutcome<'_>,
        t: &HistoryTheme,
    ) -> Vec<Section> {
        let lines = if b.name == "read" {
            read_body(o, t)
        } else {
            output_lines(
                o.text,
                self.fold_limit(),
                b.expanded,
                self.fold_takes_head(),
                t,
            )
        };
        vec![Section { label: None, lines }]
    }

    fn is_diff(&self, o: ToolOutcome<'_>) -> bool {
        ToolView::from_details(o.details, o.text).is_diff()
    }
}

pub(super) fn renderer_for(name: &str) -> &'static dyn ToolRenderer {
    match name {
        "bash" => &Bash,
        "edit" | "mass_edit" => &Edit,
        "read" | "write" | "cd" => &FileTools,
        "todo" => &Todo,
        "fetch" | "search" | "browser" => &Web,
        _ => &Fallback,
    }
}

struct Bash;

impl ToolRenderer for Bash {
    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        bash_call(b.args, t)
    }
}

struct Edit;

impl ToolRenderer for Edit {
    fn fold_limit(&self) -> usize {
        DIFF_FOLD
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        path_meta(b)
    }

    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        edit_call(b, t)
    }

    fn result_sections(
        &self,
        b: &ToolBlock<'_>,
        o: ToolOutcome<'_>,
        t: &HistoryTheme,
    ) -> Vec<Section> {
        vec![Section {
            label: None,
            lines: edit_result(o, b.expanded, t),
        }]
    }
}

struct FileTools;

impl ToolRenderer for FileTools {
    fn fold_takes_head(&self) -> bool {
        true
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let mut out = path_meta(b);
        if b.name == "read"
            && let Some(o) = b.result
            && let Some(d) = o.details
        {
            let offset = d.get("offset").and_then(|v| v.as_u64()).unwrap_or(1);
            let lines = d.get("lines").and_then(|v| v.as_u64()).unwrap_or(0);
            if offset > 1 && lines > 0 {
                let span = format!(":{}-{}", offset, offset + lines - 1);
                if let Some(first) = out.first_mut() {
                    first.push_str(&span);
                }
            }
        }
        out
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        (self.fold_takes_head() && non_empty_lines(o.text) > 1)
            .then(|| format!("{} lines", non_empty_lines(o.text)))
    }

    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        if b.name == "write" {
            write_preview(b, t)
        } else {
            Vec::new()
        }
    }
}

struct Web;

impl ToolRenderer for Web {
    fn fold_limit(&self) -> usize {
        WEB_FOLD
    }

    fn fold_takes_head(&self) -> bool {
        true
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let mut out = Vec::new();
        match b.name {
            "fetch" => {
                if let Some(u) = arg_str(b.args, "url").filter(|u| !u.trim().is_empty()) {
                    out.push(short_url(&u));
                }
                if arg_bool(b.args, "raw") == Some(true) {
                    out.push("raw".into());
                }
            }
            "search" => {
                if let Some(q) = arg_str(b.args, "query").filter(|q| !q.trim().is_empty()) {
                    out.push(truncate(&one_line(&q), 60));
                }
            }
            "browser" => {
                if let Some(c) = arg_str(b.args, "command").filter(|c| !c.trim().is_empty()) {
                    out.push(one_line(&c));
                }
                for key in ["op", "url", "path", "selector"] {
                    if let Some(v) = arg_str(b.args, key).filter(|v| !v.trim().is_empty()) {
                        out.push(truncate(&one_line(&v), 50));
                        break;
                    }
                }
            }
            _ => {}
        }
        out
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        let hits = search_hits(o.text);
        if hits > 0 {
            return Some(format!("{hits} results"));
        }
        (non_empty_lines(o.text) > 1).then(|| format!("{} lines", non_empty_lines(o.text)))
    }

    fn call_body(&self, _b: &ToolBlock<'_>, _t: &HistoryTheme) -> Vec<Line<'static>> {
        Vec::new()
    }

    fn result_sections(
        &self,
        b: &ToolBlock<'_>,
        o: ToolOutcome<'_>,
        t: &HistoryTheme,
    ) -> Vec<Section> {
        match b.name {
            "fetch" => fetch_sections(b, o.text, t),
            "search" => search_sections(b, o.text, t),
            _ => vec![Section {
                label: None,
                lines: head_preview(
                    &non_empty_text_lines(o.text),
                    WEB_FOLD,
                    WEB_OPEN,
                    b.expanded,
                    t,
                ),
            }],
        }
    }
}

struct Todo;

impl ToolRenderer for Todo {
    fn fold_limit(&self) -> usize {
        DIFF_FOLD
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let op = arg_str(b.args, "op").unwrap_or_default();
        (!op.is_empty()).then_some(op).into_iter().collect()
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        let d = o.details?;
        let (done, total) = (
            d.get("done").and_then(|v| v.as_u64())?,
            d.get("total").and_then(|v| v.as_u64())?,
        );
        Some(format!("{done}/{total} done"))
    }

    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        if b.result.is_some() {
            return Vec::new();
        }
        arg_str(b.args, "task")
            .map(|task| vec![Line::from(t.fg(format!("→ {task}"), Token::Muted))])
            .unwrap_or_default()
    }

    fn result_sections(
        &self,
        _b: &ToolBlock<'_>,
        o: ToolOutcome<'_>,
        t: &HistoryTheme,
    ) -> Vec<Section> {
        let Some(d) = o.details else {
            return Vec::new();
        };
        let Some(phases) = d.get("phases").and_then(|p| p.as_array()) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for phase in phases {
            let name = phase.get("name").and_then(|n| n.as_str()).unwrap_or("");
            lines.push(Line::from(t.fg(name.to_string(), Token::ToolTitle)));
            for task in phase
                .get("tasks")
                .and_then(|t| t.as_array())
                .into_iter()
                .flatten()
            {
                lines.push(todo_line(task, t));
            }
        }
        if lines.is_empty() {
            return Vec::new();
        }
        vec![Section { label: None, lines }]
    }
}

fn todo_line(task: &serde_json::Value, t: &HistoryTheme) -> Line<'static> {
    let status = task
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("pending");
    let content = task.get("content").and_then(|c| c.as_str()).unwrap_or("");
    let (glyph, tok) = match status {
        "done" => (g::TODO_DONE, Token::Ok),
        "in_progress" => (g::TODO_ACTIVE, Token::ToolEdgePending),
        "blocked" => (g::TODO_BLOCKED, Token::Warn),
        "abandoned" => (g::TODO_DROPPED, Token::Dim),
        _ => (g::TODO_PENDING, Token::Dim),
    };
    let body = if status == "done" {
        Token::Dim
    } else {
        Token::AssistantText
    };
    let mut spans = vec![
        t.fg(format!("{glyph} "), tok),
        t.fg(content.to_string(), body),
    ];
    if let Some(reason) = task.get("blocker").and_then(|b| b.as_str()) {
        spans.push(t.fg(format!("（卡住：{reason}）"), Token::Warn));
    }
    Line::from(spans)
}

struct Fallback;

impl ToolRenderer for Fallback {}

fn path_meta(b: &ToolBlock<'_>) -> Vec<String> {
    arg_str(b.args, "path")
        .filter(|p| !p.trim().is_empty())
        .map(|p| vec![one_line(&p)])
        .unwrap_or_default()
}

fn write_preview(b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let content = arg_str(b.args, "content").unwrap_or_default();
    if content.is_empty() {
        return Vec::new();
    }
    let lang = highlight::language_for_path(&arg_str(b.args, "path").unwrap_or_default());
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let shown = if b.expanded {
        total
    } else {
        total.min(WRITE_PREVIEW_LINES)
    };
    let gutter = total.to_string().len().max(WRITE_GUTTER_MIN_WIDTH);
    let mut out = Vec::new();
    for (i, line) in lines.iter().take(shown).enumerate() {
        let mut spans = vec![t.fg(format!("{:>gutter$} ", i + 1), Token::Dim)];
        let hl = highlight::highlight(line, lang, t);
        match hl.into_iter().next() {
            Some(l) if !l.spans.is_empty() => spans.extend(l.spans),
            _ => spans.push(t.plain(*line)),
        }
        out.push(Line::from(spans));
    }
    if total > shown {
        out.push(Line::from(
            t.fg(format!("… {} more lines", total - shown), Token::Dim),
        ));
    }
    out
}

const WRITE_PREVIEW_LINES: usize = 8;

const WRITE_GUTTER_MIN_WIDTH: usize = 3;

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

fn read_range(args: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args).ok()?;
    let offset = v.get("offset").and_then(|o| o.as_u64());
    let limit = v.get("limit").and_then(|l| l.as_u64());
    if offset.is_none() && limit.is_none() {
        return None;
    }
    let start = offset.unwrap_or(1);
    Some(match limit {
        Some(n) => format!(":{}-{}", start, start + n.saturating_sub(1)),
        None => format!(":{start}"),
    })
}

pub(super) fn read_line(
    args: &str,
    result: Option<ToolOutcome<'_>>,
    cwd: &str,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    let state = match result {
        None => State::Pending,
        Some(o) if o.ok => State::Ok,
        Some(_) => State::Err,
    };
    let mut spans = vec![
        t.fg(state.glyph(), state.glyph_token()),
        t.fg_mod(" Read", Token::ToolTitle, Modifier::BOLD),
    ];
    let path = arg_str(args, "path").unwrap_or_default();
    if !path.trim().is_empty() {
        let mut target = relativize(cwd, &path);
        if let Some(span) = read_range(args) {
            target.push_str(&span);
        }
        spans.push(t.fg(format!(" {target}"), Token::ToolPath));
    }
    let mut out = vec![Line::from(spans)];
    if let Some(o) = result.filter(|o| !o.ok) {
        for part in o.text.lines().filter(|s| !s.trim().is_empty()) {
            out.push(Line::from(t.fg(format!("  {part}"), Token::Err)));
        }
    }
    out
}

pub(super) fn cd_line(
    args: &str,
    result: Option<ToolOutcome<'_>>,
    cwd: &str,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    let state = match result {
        None => State::Pending,
        Some(o) if o.ok => State::Ok,
        Some(_) => State::Err,
    };
    let landed = result
        .and_then(|o| o.details)
        .filter(|d| d.get("kind").and_then(|k| k.as_str()) == Some("cwd"))
        .and_then(|d| d.get("path").and_then(|p| p.as_str()).map(String::from));
    let want = landed.or_else(|| arg_str(args, "path"));
    let base = result
        .and_then(|o| o.details)
        .and_then(|d| d.get("from").and_then(|p| p.as_str()).map(String::from))
        .unwrap_or_else(|| cwd.to_string());
    let mut spans = vec![
        t.fg(state.glyph(), state.glyph_token()),
        t.fg_mod(" Cd", Token::ToolTitle, Modifier::BOLD),
    ];
    if let Some(p) = want.filter(|p| !p.trim().is_empty()) {
        let tok = if matches!(state, State::Ok) {
            Token::ToolPath
        } else {
            Token::Err
        };
        spans.push(t.fg(format!(" {}", relativize(&base, &p)), tok));
    }
    vec![Line::from(spans)]
}

fn read_body(o: ToolOutcome<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut numbered: Vec<(usize, String)> = Vec::new();
    for line in o.text.lines() {
        let parsed = line.trim_start().split_once('\t').and_then(|(n, text)| {
            n.parse::<usize>()
                .ok()
                .map(|n| (n, text.replace('\t', "    ")))
        });
        match parsed {
            Some((n, text)) => numbered.push((n, text)),
            None => {
                flush_numbered(&mut numbered, &mut out, t);
                out.push(Line::from(t.fg(line.to_string(), Token::Muted)));
            }
        }
    }
    flush_numbered(&mut numbered, &mut out, t);
    out
}

fn flush_numbered(
    numbered: &mut Vec<(usize, String)>,
    out: &mut Vec<Line<'static>>,
    t: &HistoryTheme,
) {
    if numbered.is_empty() {
        return;
    }
    let width = numbered
        .iter()
        .map(|(n, _)| n.to_string().len())
        .max()
        .unwrap_or(1)
        .max(WRITE_GUTTER_MIN_WIDTH);
    for (n, text) in numbered.drain(..) {
        let mut spans = vec![t.fg(format!("{n:>width$}│"), Token::Dim)];
        let hl = highlight::highlight(&text, None, t);
        match hl.into_iter().next() {
            Some(l) if !l.spans.is_empty() => spans.extend(l.spans),
            _ => spans.push(t.plain(text)),
        }
        out.push(Line::from(spans));
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ToolBlock<'a> {
    pub name: &'a str,
    pub args: &'a str,
    pub result: Option<ToolOutcome<'a>>,
    pub expanded: bool,
}

impl<'a> ToolBlock<'a> {
    pub(super) fn state(&self) -> State {
        match self.result {
            None => State::Pending,
            Some(o) if o.ok => State::Ok,
            Some(_) => State::Err,
        }
    }

    pub(super) fn edge_style(&self, t: &HistoryTheme) -> Style {
        t.fg_style(self.state().edge_token())
    }

    pub(super) fn bg_style(&self, t: &HistoryTheme) -> Style {
        Style::new().bg(t.get(self.state().bg_token()))
    }

    pub(super) fn header(&self, t: &HistoryTheme) -> Line<'static> {
        let state = self.state();
        let mut spans = vec![
            t.fg(state.glyph(), state.glyph_token()),
            t.fg_mod(format!(" {}", self.name), Token::ToolTitle, Modifier::BOLD),
        ];
        if let Some(d) = description(self.args) {
            spans.push(t.fg(format!(": {d}"), Token::Muted));
        }
        for m in self.meta() {
            spans.push(t.fg(format!(" {} {m}", g::SEP), Token::Dim));
        }
        Line::from(spans)
    }

    fn r(&self) -> &'static dyn ToolRenderer {
        renderer_for(self.name)
    }

    fn meta(&self) -> Vec<String> {
        let mut out = self.r().meta(self);
        if let Some(o) = self.result {
            if let Some(m) = self.r().result_measure(o) {
                out.push(m);
            }
            if let Some(d) = o.duration_label() {
                out.push(d);
            }
        }
        out
    }

    pub(super) fn call_body(&self, t: &HistoryTheme) -> Vec<Line<'static>> {
        self.r().call_body(self, t)
    }

    pub(super) fn sections(&self, t: &HistoryTheme) -> Vec<Section> {
        let mut out = Vec::new();
        let call = self.call_body(t);
        if !call.is_empty() {
            out.push(Section {
                label: None,
                lines: call,
            });
        }
        if let Some(o) = self.result {
            out.extend(self.result_sections(o, t));
        }
        out.retain(|s| !s.lines.is_empty());
        out
    }

    fn result_sections(&self, o: ToolOutcome<'_>, t: &HistoryTheme) -> Vec<Section> {
        self.r().result_sections(self, o, t)
    }

    pub(super) fn has_two_states(&self) -> bool {
        let Some(o) = self.result else {
            return false;
        };
        if self.r().is_diff(o) {
            return false;
        }
        non_empty_lines(o.text) > self.r().fold_limit()
    }
}

fn description(args: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args).ok()?;
    let intent = v.get("intent")?.as_str()?.trim();
    if intent.is_empty() {
        return None;
    }
    Some(one_line(intent))
}

fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

fn bash_call(args: &str, t: &HistoryTheme) -> Vec<Line<'static>> {
    let cmd = arg_str(args, "command").unwrap_or_default();
    if cmd.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, line) in cmd.lines().enumerate() {
        let mut spans = Vec::new();
        if i == 0 {
            spans.push(t.fg("$ ", Token::Dim));
        }
        let hl = highlight::highlight(line, Some("sh"), t);
        match hl.into_iter().next() {
            Some(l) if !l.spans.is_empty() => spans.extend(l.spans),
            _ => spans.push(t.plain(line)),
        }
        out.push(Line::from(spans));
    }
    out
}

fn edit_call(b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
    if b.result.is_some() {
        return Vec::new();
    }
    let path = arg_str(b.args, "path").unwrap_or_default();
    let lang = highlight::language_for_path(&path);
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
            t,
        );
    }
    if let Some(new) = new_text {
        push_diff(&mut out, '+', &new, changed.map(|(_, r)| r), lang, t);
    }
    if out.is_empty()
        && let Some(edits) = serde_json::from_str::<serde_json::Value>(b.args)
            .ok()
            .and_then(|v| v.get("edits").and_then(|e| e.as_array()).cloned())
    {
        let mut text = String::new();
        for e in edits {
            let line = e.get("line").and_then(|l| l.as_i64()).unwrap_or(0);
            let body = e.get("text").and_then(|l| l.as_str()).unwrap_or("");
            text.push_str(&format!("{line}: {body}\n"));
        }
        push_diff(&mut out, '+', text.trim_end(), None, lang, t);
    }
    out
}

fn changed_spans(old: &str, new: &str) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
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

fn split_at_bytes(
    spans: Vec<Span<'static>>,
    lo: usize,
    hi: usize,
) -> (Vec<Span<'static>>, Vec<Span<'static>>, Vec<Span<'static>>) {
    let mut before = Vec::new();
    let mut mid = Vec::new();
    let mut after = Vec::new();
    let mut at = 0usize;
    for sp in spans {
        let (start, end) = (at, at + sp.content.len());
        at = end;
        for (lo, hi, bucket) in [
            (0, lo, &mut before),
            (lo, hi, &mut mid),
            (hi, usize::MAX, &mut after),
        ] {
            let (s, e) = (start.max(lo), end.min(hi));
            if s < e {
                bucket.push(Span::styled(
                    sp.content[(s - start)..(e - start)].to_string(),
                    sp.style,
                ));
            }
        }
    }
    (before, mid, after)
}

fn push_diff(
    out: &mut Vec<Line<'static>>,
    sign: char,
    text: &str,
    changed: Option<std::ops::Range<usize>>,
    lang: Option<&str>,
    t: &HistoryTheme,
) {
    let tok = if sign == '-' {
        Token::DiffRemoved
    } else {
        Token::DiffAdded
    };
    for (i, l) in text.lines().enumerate() {
        let mut spans = vec![t.fg(format!("{sign} "), tok)];
        let hl = highlight::highlight(l, lang, t)
            .into_iter()
            .next()
            .filter(|ln| !ln.spans.is_empty());
        let r = if i == 0 { changed.clone() } else { None };
        match (hl, r) {
            (Some(ln), Some(r)) => {
                let (pre, mid, post) = split_at_bytes(ln.spans, r.start, r.end);
                spans.extend(pre);
                for sp in mid {
                    spans.push(Span::styled(
                        sp.content,
                        sp.style.add_modifier(Modifier::REVERSED),
                    ));
                }
                spans.extend(post);
            }
            (Some(ln), None) => spans.extend(ln.spans),
            (None, _) => spans.push(t.plain(l)),
        }
        out.push(Line::from(spans));
    }
}

fn edit_result(o: ToolOutcome<'_>, expanded: bool, t: &HistoryTheme) -> Vec<Line<'static>> {
    match ToolView::from_details(o.details, o.text) {
        ToolView::Diff {
            deletions,
            insertions,
            at_line,
        } => {
            let card = t.get(Token::ToolBgSuccess);
            let fg = contrast_on(card);
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
            let mut emit = |sign: char,
                            idx: usize,
                            text: &str,
                            tok: Token,
                            r: Option<std::ops::Range<usize>>| {
                let n = at_line.map(|at| at + idx);
                let blank = matches!((n, last_shown), (Some(a), Some(b)) if a == b);
                last_shown = n;
                let style = Style::new().fg(fg).bg(dim_bg(t.get(tok), card));
                let lead = gutter(sign, idx, blank);
                match r {
                    Some(r) => out.push(Line::from(vec![
                        Span::styled(format!("{lead}{}", &text[..r.start]), style),
                        Span::styled(
                            text[r.start..r.end].to_string(),
                            style.add_modifier(Modifier::REVERSED),
                        ),
                        Span::styled(text[r.end..].to_string(), style),
                    ])),
                    None => out.push(Line::styled(format!("{lead}{text}"), style)),
                }
            };
            for (i, d) in deletions.iter().enumerate() {
                emit(
                    '-',
                    i,
                    d,
                    Token::DiffRemoved,
                    changed.as_ref().map(|(r, _)| r.clone()),
                );
            }
            for (i, ins) in insertions.iter().enumerate() {
                emit(
                    '+',
                    i,
                    ins,
                    Token::DiffAdded,
                    changed.as_ref().map(|(_, r)| r.clone()),
                );
            }
            out
        }
        ToolView::Plain { text } => output_lines(&text, DIFF_FOLD, expanded, false, t),
    }
}

pub(super) fn output_lines(
    text: &str,
    limit: usize,
    expanded: bool,
    takes_head: bool,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    if text.trim().is_empty() {
        return vec![Line::from(t.fg("(no output)", Token::Muted))];
    }
    let lines: Vec<&str> = text.lines().collect();
    let fold = !expanded && lines.len() > limit;
    let hidden = lines.len().saturating_sub(limit);
    let marker = if takes_head {
        format!("… {hidden} more lines {}", g::EXPAND_HINT)
    } else {
        format!("… {hidden} earlier lines {}", g::EXPAND_HINT)
    };
    let kept: &[&str] = if !fold {
        &lines[..]
    } else if takes_head {
        &lines[..limit]
    } else {
        &lines[hidden..]
    };
    let mut out = Vec::new();
    if fold && takes_head {
        for l in kept {
            out.push(Line::from(t.fg(*l, Token::ToolOutput)));
        }
        out.push(Line::from(t.fg(marker, Token::Dim)));
        return out;
    }
    if fold {
        out.push(Line::from(t.fg(marker, Token::Dim)));
    }
    for l in kept {
        out.push(Line::from(t.fg(*l, Token::ToolOutput)));
    }
    out
}

fn head_preview(
    lines: &[String],
    collapsed: usize,
    cap: usize,
    expanded: bool,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    let limit = if expanded { cap } else { collapsed };
    if lines.is_empty() {
        return vec![Line::from(t.fg("(no content)", Token::Muted))];
    }
    let mut out: Vec<Line<'static>> = lines
        .iter()
        .take(limit)
        .map(|l| Line::from(t.fg(l.clone(), Token::ToolOutput)))
        .collect();
    if lines.len() > limit {
        out.push(Line::from(t.fg(
            format!("… {} more lines {}", lines.len() - limit, g::EXPAND_HINT),
            Token::Dim,
        )));
    }
    out
}

fn fetch_sections(b: &ToolBlock<'_>, text: &str, t: &HistoryTheme) -> Vec<Section> {
    let url = arg_str(b.args, "url").unwrap_or_default();
    let raw = arg_bool(b.args, "raw") == Some(true);
    let lines = non_empty_text_lines(text);
    let chars = text.trim().chars().count();

    let mut meta: Vec<Line<'static>> = Vec::new();
    if !url.trim().is_empty() {
        meta.push(fact("URL: ", &truncate(&one_line(&url), 80), t));
    }
    meta.push(fact("Mode: ", if raw { "raw HTML" } else { "markdown" }, t));
    meta.push(fact("Lines: ", &lines.len().to_string(), t));
    meta.push(fact("Chars: ", &chars.to_string(), t));

    vec![
        Section {
            label: Some("metadata"),
            lines: meta,
        },
        Section {
            label: Some("content"),
            lines: head_preview(&lines, WEB_FOLD, WEB_OPEN, b.expanded, t),
        },
    ]
}

fn search_sections(b: &ToolBlock<'_>, text: &str, t: &HistoryTheme) -> Vec<Section> {
    let query = arg_str(b.args, "query").unwrap_or_default();
    let lines = non_empty_text_lines(text);
    let mut out = Vec::new();
    if !query.trim().is_empty() {
        out.push(Section {
            label: None,
            lines: vec![fact("Query: ", &one_line(&query), t)],
        });
    }
    out.push(Section {
        label: Some("results"),
        lines: head_preview(&lines, WEB_FOLD, WEB_OPEN, b.expanded, t),
    });
    out
}

fn fact(label: &str, value: &str, t: &HistoryTheme) -> Line<'static> {
    Line::from(vec![
        t.fg(label, Token::Muted),
        t.fg(value.to_string(), Token::ToolOutput),
    ])
}

fn default_call(name: &str, args: &str, t: &HistoryTheme) -> Vec<Line<'static>> {
    let lang = highlight::language_for_tool(name);
    match serde_json::from_str::<serde_json::Value>(args) {
        Ok(v) => {
            let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| args.to_string());
            let mut out = Vec::new();
            for l in pretty.lines() {
                out.extend(highlight::highlight(l, Some("json"), t));
            }
            out
        }
        Err(_) => {
            let mut out = Vec::new();
            for l in args.lines() {
                out.extend(highlight::highlight(l, lang, t));
            }
            out
        }
    }
}

fn non_empty_lines(text: &str) -> usize {
    non_empty_text_lines(text).len()
}

fn non_empty_text_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

fn search_hits(text: &str) -> usize {
    text.lines()
        .filter(|l| {
            let l = l.trim_start();
            let digits: String = l.chars().take_while(char::is_ascii_digit).collect();
            !digits.is_empty() && l[digits.len()..].starts_with(". ")
        })
        .count()
}

fn short_url(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    truncate(&one_line(rest), 60)
}

fn truncate(text: &str, max: usize) -> String {
    let (cut, w) = crate::ui::tui::text::take_width(text, max);
    if w < unicode_width::UnicodeWidthStr::width(text) {
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

fn arg_bool(args: &str, key: &str) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_str(args).ok()?;
    v.get(key)?.as_bool()
}

fn arg_str(args: &str, key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args).ok()?;
    Some(v.get(key)?.as_str()?.to_string())
}

impl ToolView {
    pub(super) fn is_diff(&self) -> bool {
        matches!(self, ToolView::Diff { .. })
    }
}

fn contrast_on(bg: Color) -> Color {
    crate::ui::tui::theme::contrast_text_on(bg)
}

fn dim_bg(fg: Color, card: Color) -> Color {
    let (fr, fg_, fb) = match fg {
        Color::Rgb(r, g, b) => (r as u32, g as u32, b as u32),
        _ => return card,
    };
    let (cr, cg, cb) = match card {
        Color::Rgb(r, g, b) => (r as u32, g as u32, b as u32),
        _ => return card,
    };
    Color::Rgb(
        ((cr * 3 + fr) / 4) as u8,
        ((cg * 3 + fg_) / 4) as u8,
        ((cb * 3 + fb) / 4) as u8,
    )
}
