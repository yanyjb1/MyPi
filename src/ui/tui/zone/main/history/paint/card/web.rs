//! web: a fetch's facts and body, or a search's query and hits.

use super::super::super::line::Row;
use super::{
    Section, ToolBlock, ToolOutcome, ToolRenderer, arg_bool, arg_str, fact, head_preview,
    non_empty_lines, non_empty_text_lines, one_line, search_hits, short_url, truncate,
};

pub(crate) struct Web;

impl ToolRenderer for Web {
    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let mut out = Vec::new();
        match arg_str(b.args, "op").as_deref().unwrap_or_default() {
            "read" => {
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
            op if !op.is_empty() => {
                for key in ["url", "path", "selector"] {
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

    fn call_body(&self, _b: &ToolBlock<'_>) -> Vec<Row> {
        Vec::new()
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        match arg_str(b.args, "op").as_deref().unwrap_or_default() {
            "read" => fetch_sections(b, o.text),
            "search" => search_sections(b, o.text),
            _ => vec![Section {
                label: None,
                lines: head_preview(&non_empty_text_lines(o.text)),
            }],
        }
    }
}

fn fetch_sections(b: &ToolBlock<'_>, text: &str) -> Vec<Section> {
    let url = arg_str(b.args, "url").unwrap_or_default();
    let raw = arg_bool(b.args, "raw") == Some(true);
    let lines = non_empty_text_lines(text);
    let chars = text.trim().chars().count();

    let mut meta: Vec<Row> = Vec::new();
    if !url.trim().is_empty() {
        meta.push(fact("URL: ", &truncate(&one_line(&url), 80)));
    }
    meta.push(fact("Mode: ", if raw { "raw HTML" } else { "markdown" }));
    meta.push(fact("Lines: ", &lines.len().to_string()));
    meta.push(fact("Chars: ", &chars.to_string()));

    vec![
        Section {
            label: Some("metadata"),
            lines: meta,
        },
        Section {
            label: Some("content"),
            lines: head_preview(&lines),
        },
    ]
}

fn search_sections(b: &ToolBlock<'_>, text: &str) -> Vec<Section> {
    let query = arg_str(b.args, "query").unwrap_or_default();
    let lines = non_empty_text_lines(text);
    let mut out = Vec::new();
    if !query.trim().is_empty() {
        out.push(Section {
            label: None,
            lines: vec![fact("Query: ", &one_line(&query))],
        });
    }
    out.push(Section {
        label: Some("results"),
        lines: head_preview(&lines),
    });
    out
}
