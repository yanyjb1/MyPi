//! web: `search` as a card, every other operation as one line.

use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::tree;
use super::{
    Section, State, ToolBlock, ToolOutcome, ToolRenderer, arg_bool, arg_str, error_body,
    error_under, head, non_empty_lines, one_line, short_url, truncate,
};
use crate::ui::tui::text::{display_width, glyphs as g};
use crate::ui::tui::theme::fixed;

/// The one-line form; `search` keeps a card.
pub(crate) fn inline(args: &str, result: Option<ToolOutcome<'_>>) -> Option<Vec<Row>> {
    let op = op_of(args);
    if op == "search" {
        return None;
    }
    let state = State::of(result);
    let mut runs = head(state, &label_of(&op));
    let mut meta: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    match op.as_str() {
        "read" => {
            let target = arg_str(args, "url")
                .filter(|u| !u.trim().is_empty())
                .map(|u| short_url(&u))
                .unwrap_or_else(|| "page".to_string());
            values.push(target);
            if arg_bool(args, "raw") == Some(true) {
                meta.push("raw".to_string());
            }
            if let Some(o) = result.filter(|o| o.ok == Some(true)) {
                meta.push(format!("{} lines", non_empty_lines(o.text)));
            }
        }
        "open" => {
            if let Some(u) = arg_str(args, "url").filter(|u| !u.trim().is_empty()) {
                values.push(short_url(&u));
            }
        }
        "click" | "fill" | "press" | "select" => {
            if let Some(sel) = arg_str(args, "selector").filter(|s| !s.trim().is_empty()) {
                values.push(truncate(&one_line(&sel), 48));
            }
        }
        "scroll" => {
            values.push(format!(
                "{}px",
                value_of(args, "value").unwrap_or_else(|| "600".to_string())
            ));
        }
        "eval" => {
            if let Some(expr) = value_of(args, "value").filter(|v| !v.trim().is_empty()) {
                values.push(truncate(&one_line(&expr), 48));
            }
        }
        "screenshot" => {
            if arg_bool(args, "full_page") == Some(true) {
                meta.push("full page".to_string());
            }
            if let Some(o) = result.filter(|o| o.ok == Some(true))
                && let Some(path) = o.text.split(" -> ").nth(1)
            {
                values.push(truncate(&one_line(path), 60));
            }
        }
        other if !other.is_empty() => values.push(other.to_string()),
        _ => {}
    }
    if !values.is_empty() {
        runs.push(Run::fixed(format!(" {}", values.join(" ")), fixed::MUTED));
    }
    for m in meta {
        runs.push(Run::fixed(format!(" {} {m}", g::SEP), fixed::DIM));
    }
    let mut out = vec![Row::of(Lead::None, runs)];
    out.extend(error_under(result));
    Some(out)
}

pub(crate) struct Web;

impl ToolRenderer for Web {
    fn label(&self, b: &ToolBlock<'_>) -> Option<String> {
        match op_of(b.args).as_str() {
            "search" => Some("Web Search".to_string()),
            _ => None,
        }
    }

    fn title(&self, b: &ToolBlock<'_>) -> Option<String> {
        let text = b.result.map(|o| o.text).unwrap_or_default();
        match op_of(b.args).as_str() {
            "search" => {
                let mut parts: Vec<String> = Vec::new();
                if let Some(q) = arg_str(b.args, "query").filter(|q| !q.trim().is_empty()) {
                    parts.push(truncate(&one_line(&q), 60));
                }
                if let Some(p) = arg_str(b.args, "provider").filter(|p| !p.trim().is_empty()) {
                    parts.push(p);
                }
                let count = hits(text).len();
                if count > 0 {
                    parts.push(format!("{count} results"));
                }
                (!parts.is_empty()).then(|| parts.join(" · "))
            }
            _ => None,
        }
    }

    fn call_body(&self, _b: &ToolBlock<'_>) -> Vec<Row> {
        Vec::new()
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        if o.ok == Some(false) {
            return vec![Section {
                label: None,
                lines: error_body(o.text),
            }];
        }
        let lines = match op_of(b.args).as_str() {
            "search" => search_lines(hits(o.text), b.inner_width()),
            _ => Vec::new(),
        };
        vec![Section { label: None, lines }]
    }
}

fn op_of(args: &str) -> String {
    arg_str(args, "op").unwrap_or_default()
}

fn label_of(op: &str) -> String {
    let mut chars = op.chars();
    match chars.next() {
        Some(c) => format!("Web {}{}", c.to_uppercase(), chars.as_str()),
        None => "Web".to_string(),
    }
}

/// A scalar argument spelled out, strings or numbers.
fn value_of(args: &str, key: &str) -> Option<String> {
    let v: Value = serde_json::from_str(args).ok()?;
    match v.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A hit's title and URL, in the text `web search` prints.
fn hits(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let Some(title) = numbered(line) else {
            continue;
        };
        let url = lines[at + 1..]
            .iter()
            .take_while(|next| numbered(next).is_none())
            .map(|next| next.trim())
            .find(|next| next.starts_with("http"))
            .unwrap_or_default();
        out.push((title, url.to_string()));
    }
    out
}

/// The text of a `N. text` line; a line without a number answers nothing.
fn numbered(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || !trimmed[digits..].starts_with(". ") {
        return None;
    }
    Some(trimmed[digits + 2..].to_string())
}

fn domain_of(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host = rest.split('/').next().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// One branch per hit: the title, then the domain it came from.
fn search_lines(hits: Vec<(String, String)>, inner: usize) -> Vec<Row> {
    let last = hits.len().saturating_sub(1);
    hits.iter()
        .enumerate()
        .map(|(at, (title, url))| {
            let domain = domain_of(url);
            let tail = if domain.is_empty() {
                String::new()
            } else {
                format!(" ({domain})")
            };
            let budget = inner.saturating_sub(4 + display_width(&tail) + 1).max(12);
            let mut runs = vec![Run::fixed(truncate(title, budget), fixed::TOOL_OUTPUT)];
            if !tail.is_empty() {
                runs.push(tree::value(tail));
            }
            tree::entry(at == last, runs)
        })
        .collect()
}
