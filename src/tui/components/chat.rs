//! Chat history: typed entries + rendering dispatched by kind.
//!
//! An early version stored messages as "prefix + body" strings in `Vec<String>`,
//! with renderers guessing the type via strip_prefix — a REPL-era relic,
//! now fully retired. History is an array of [`Entry`]; rendering dispatches on kind.
//!
//! Data flow: streaming writes only the in-memory in-progress slot; at TurnDone the
//! whole round's entries are persisted in one shot. Rendering only accepts `&[Entry]`,
//! so in-memory and DB-sourced history share one render path.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::tui::theme::Palette;
use unicode_width::UnicodeWidthStr;

// One history entry: the four message kinds plus a session-level error.
//
// Persisted to the SQLite `entries` table: `seq` monotonically increasing from 1,
// `kind` stored as text, `payload` as JSON. The render layer only knows this enum —
// no string-prefix contracts anymore.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    // A user message.
    User { content: String },
    // Model reply (final). `usage` feeds the stats line; optional.
    // `reasoning` is the thinking chain (display + persistence only; never sent back in requests).
    Assistant {
        content: String,
        usage: Option<UsageSummary>,
        reasoning: Option<String>,
    },
    // Tool call request: the model named a tool. Displays the object argument (e.g. a path).
    // `call_id` is the protocol pairing key (persisted; resume rebuilds protocol messages from it).
    ToolRequest { call_id: String, name: String, object: String },
    // Tool call result. `ok` decides the card color.
    // `result` is the **exact text sent to the model** — the only thing persisted;
    // the rendering (view) is **synthesized at render time** from (name, ok, result), never stored.
    ToolResult {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
    },
    // Session-level errors (HTTP failures, round-limit brakes...). Not persisted; memory stream only.
    Error { text: String },
    // Session name marker on the conversation tree (pi's session_info). Persisted;
    // the effective name is the nearest `name` entry looking back from the leaf,
    // so branches inherit the name and renaming only affects the current branch.
    Name { name: String },
}

// The tool result's view for the UI. **The model only ever receives plain text**;
// this enum only declares how the UI draws it — no protocol role, **never persisted** —
// synthesized at render time by `synthesize` from (tool name, ok, result text).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ToolView {
    // Plain text, no markdown rendering, auto-folded beyond 5 lines.
    Plain { text: String },
    // Line diff: deletions on red, insertions on green (edit tool).
    Diff { deletions: Vec<String>, insertions: Vec<String> },
}

impl ToolView {
    // Synthesize the UI presentation from data (tool name / ok / model-facing text).
    //
    // The single source of the synthesis rules. Only the result text is persisted;
    // the view is a derived UI concept and takes no part in persistence.
    pub fn synthesize(name: &str, ok: bool, result: &str) -> ToolView {
        if !ok {
            return ToolView::Plain { text: result.to_string() };
        }
        match name {
            "edit" | "mass_edit" => {
                // Diff-shaped text (- / + lines) splits into a Diff; otherwise plain text
                let mut deletions = Vec::new();
                let mut insertions = Vec::new();
                for line in result.lines() {
                    if let Some(rest) = line.strip_prefix("- ") {
                        deletions.push(rest.to_string());
                    } else if let Some(rest) = line.strip_prefix("+ ") {
                        insertions.push(rest.to_string());
                    }
                }
                if deletions.is_empty() && insertions.is_empty() {
                    ToolView::Plain { text: result.to_string() }
                } else {
                    ToolView::Diff { deletions, insertions }
                }
            }
            _ => ToolView::Plain { text: result.to_string() },
        }
    }
}

// Fields the stats line needs (the minimal set extracted from usage).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UsageSummary {
    pub total_tokens: u64,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
}

impl Entry {
    // Build a stats summary from the ai layer's usage.
    pub fn usage_summary(u: &crate::ai::types::Usage) -> UsageSummary {
        UsageSummary {
            total_tokens: u.total_tokens,
            prompt_tokens: u.prompt_tokens,
            cached_tokens: u.cached_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens,
            reasoning_tokens: u.reasoning_tokens.unwrap_or(0),
        }
    }

    // Serialize to (kind, payload_json). Used for DB writes.
    pub fn to_payload(&self) -> (&'static str, String) {
        match self {
            Entry::User { content } => ("user", serde_json::json!({ "content": content }).to_string()),
            Entry::Assistant { content, usage, reasoning } => (
                "assistant",
                serde_json::json!({ "content": content, "usage": usage, "reasoning": reasoning })
                    .to_string(),
            ),
            Entry::ToolRequest { call_id, name, object } => (
                "tool_request",
                serde_json::json!({ "call_id": call_id, "name": name, "object": object }).to_string(),
            ),
            Entry::ToolResult { call_id, name, ok, result } => (
                "tool_result",
                serde_json::json!({ "call_id": call_id, "name": name, "ok": ok, "result": result }).to_string(),
            ),
            Entry::Error { text } => ("error", serde_json::json!({ "text": text }).to_string()),
            Entry::Name { name } => ("name", serde_json::json!({ "name": name }).to_string()),
        }
    }

    // Restore from (kind, payload_json). Used for DB reads.
    pub fn from_payload(kind: &str, payload: &str) -> Option<Entry> {
        let v: serde_json::Value = serde_json::from_str(payload).ok()?;
        Some(match kind {
            "user" => Entry::User {
                content: v.get("content")?.as_str()?.to_string(),
            },
            "assistant" => Entry::Assistant {
                content: v.get("content")?.as_str()?.to_string(),
                usage: v.get("usage").and_then(|u| serde_json::from_value(u.clone()).ok()),
                reasoning: v.get("reasoning").and_then(|r| r.as_str()).map(String::from),
            },
            "tool_request" => Entry::ToolRequest {
                call_id: v.get("call_id").and_then(|c| c.as_str()).unwrap_or_default().to_string(),
                name: v.get("name")?.as_str()?.to_string(),
                object: v.get("object")?.as_str()?.to_string(),
            },
            "tool_result" => Entry::ToolResult {
                call_id: v.get("call_id").and_then(|c| c.as_str()).unwrap_or_default().to_string(),
                name: v.get("name")?.as_str()?.to_string(),
                ok: v.get("ok")?.as_bool()?,
                // Legacy DBs store view without result: recover from view once (migration path)
                result: v.get("result").and_then(|r| r.as_str()).map(String::from).unwrap_or_else(|| {
                    let view = v.get("view").cloned();
                    match view.and_then(|view| serde_json::from_value::<ToolView>(view).ok()) {
                        Some(ToolView::Plain { text }) => text,
                        Some(ToolView::Diff { deletions, insertions }) => {
                            let mut t = String::new();
                            for d in &deletions {
                                t.push_str("- ");
                                t.push_str(d);
                                t.push('\n');
                            }
                            for i in &insertions {
                                t.push_str("+ ");
                                t.push_str(i);
                            }
                            t
                        }
                        None => String::new(),
                    }
                }),
            },
            "error" => Entry::Error {
                text: v.get("text")?.as_str()?.to_string(),
            },
            "name" => Entry::Name {
                name: v.get("name")?.as_str()?.to_string(),
            },
            _ => return None,
        })
    }
}

// Render entries into ratatui lines.
//
// The single render entry point: in-memory history and DB-resumed history
// both go through it, guaranteeing "reopened after persistence" looks identical to "just typed".
pub fn render(entries: &[Entry], p: &Palette, show_reasoning: bool, tools_expanded: bool) -> Vec<Line<'static>> {
    render_with_live(entries, p, show_reasoning, tools_expanded, None, false, None)
}

// Render plus the streaming tail (in-flight reasoning/content). Tests and resume use [`render`].
#[allow(clippy::too_many_arguments)]
pub fn render_with_live(
    entries: &[Entry],
    p: &Palette,
    show_reasoning: bool,
    tools_expanded: bool,
    live_reasoning: Option<&str>,
    reasoning_done: bool,
    streaming: Option<&str>,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            Entry::User { content } => out.extend(user_block(content, p)),
            Entry::Assistant { content, usage, reasoning } => {
                out.extend(assistant_block(content, reasoning.as_deref(), usage.as_ref(), p, show_reasoning))
            }
            Entry::ToolRequest { name, object, .. } => out.push(Line::styled(
                format!("⚙ {name} →{object}"),
                Style::new().fg(p.muted),
            )),
            Entry::ToolResult { name, ok, result, .. } => {
                // The view is not persisted: synthesized from data at render time
                let view = ToolView::synthesize(name, *ok, result);
                out.extend(tool_result_block(name, *ok, &view, p, tools_expanded))
            }
            Entry::Error { text } => {
                for part in text.split('\n') {
                    out.push(Line::styled(part.to_string(), Style::new().fg(Color::Red)));
                }
            }
            // Name markers are metadata, not chat content: never a history row.
            Entry::Name { .. } => {}
        }
    }
    // ---- streaming tail ----
    // Reasoning (content not yet started): one "thinking" line, gray italic.
    // Once content starts (reasoning_done) the line is withdrawn — content takes its place.
    if let Some(r) = live_reasoning
        && !reasoning_done
        && !r.trim().is_empty()
    {
        out.push(Line::styled("thinking", Style::new().fg(p.muted).add_modifier(Modifier::ITALIC)));
    }
    // In-flight content: gray italic would be wrong (it is the final answer); default style, appended per delta
    if let Some(t) = streaming
        && !t.is_empty()
    {
        out.extend(super::markdown::render_markdown(t, p));
    }
    out
}

// User message: markdown rendered, accent vertical bar on the left.
fn user_block(content: &str, p: &Palette) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for mut line in super::markdown::render_markdown(content, p) {
        line.spans.insert(0, Span::styled("▌ ", Style::new().fg(p.accent)));
        out.push(line);
    }
    if !out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

// Model reply: markdown rendered + stats line (gray) + a separating blank line.
fn assistant_block(
    content: &str,
    reasoning: Option<&str>,
    usage: Option<&UsageSummary>,
    p: &Palette,
    show_reasoning: bool,
) -> Vec<Line<'static>> {
    // usage is persisted with the entry (billing/stats) and never rendered as fine print in chat
    let _ = usage;
    let mut out = Vec::new();
    // Reasoning block: shown only in the Ctrl+T expanded state, gray italic, no prefix, no emoji;
    // folded (default) it is hidden entirely — content starts right where the reasoning sits.
    // The request layer strips it before sending (DeepSeek official endpoint excepted; see client docs).
    if show_reasoning && let Some(r) = reasoning.filter(|r| !r.trim().is_empty()) {
        for mut line in super::markdown::render_markdown(r, p) {
            for sp in &mut line.spans {
                sp.style = sp.style.fg(p.muted).add_modifier(Modifier::ITALIC);
            }
            out.push(line);
        }
        out.push(Line::from(""));
    }
    out.extend(super::markdown::render_markdown(content, p));
    out.push(Line::from(""));
    out
}

// Default fold threshold for tool output (lines). Per-tool overrides: [`collapse_limit`].
const DEFAULT_COLLAPSE_LINES: usize = 5;

// Per-tool thresholds: edit tools are lenient (a screenful of diff is worth showing directly).
// Unregistered tools fall back to the default 5 lines.
fn collapse_limit(tool: &str) -> usize {
    match tool {
        "edit" | "mass_edit" => 14,
        _ => DEFAULT_COLLAPSE_LINES,
    }
}

// Tool result card: ✓/✗ + tool name, body drawn per the tool's view.
// `expanded` is the global switch (Ctrl+O): true ignores thresholds and expands everything.
fn tool_result_block(name: &str, ok: bool, view: &ToolView, p: &Palette, expanded: bool) -> Vec<Line<'static>> {
    let mark = if ok { "✓" } else { "✗" };
    let head_style = if ok {
        Style::new().fg(Color::Green)
    } else {
        Style::new().fg(Color::Red)
    };
    let mut out = vec![Line::styled(format!("{mark} {name}"), head_style)];
    match view {
        ToolView::Plain { text } => {
            let lines: Vec<&str> = text.lines().collect();
            let limit = collapse_limit(name);
            // Over threshold: show the first limit rows + a hint line (Ctrl+O expands globally)
            if !expanded && lines.len() > limit {
                for l in &lines[..limit] {
                    out.push(Line::styled(format!("  {l}"), Style::new().fg(p.muted)));
                }
                out.push(Line::styled(
                    format!("  … 共 {} 行", lines.len()),
                    Style::new().fg(p.muted),
                ));
            } else {
                for l in lines {
                    out.push(Line::styled(format!("  {l}"), Style::new().fg(p.muted)));
                }
            }
        }
        ToolView::Diff { deletions, insertions } => {
            // Deletions above, insertions below: red and green backgrounds (vscode style)
            for d in deletions {
                out.push(Line::styled(
                    format!("- {d}"),
                    Style::new().fg(Color::Black).bg(Color::Rgb(255, 128, 128)),
                ));
            }
            for i in insertions {
                out.push(Line::styled(
                    format!("+ {i}"),
                    Style::new().fg(Color::Black).bg(Color::Rgb(128, 200, 128)),
                ));
            }
        }
    }
    out
}

// Estimate the rendered visual line count (used to compute scrolling).
pub fn estimated_height(lines: &[Line], width: usize) -> usize {
    let w = width.max(1);
    lines
        .iter()
        .map(|l| {
            let tw: usize = l.spans.iter().map(|sp| sp.content.as_ref().width()).sum();
            if tw == 0 { 1 } else { tw.div_ceil(w) }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_render_by_kind_not_by_prefix() {
        let p = Palette::default();
        let entries = vec![
            Entry::User { content: "问题".into() },
            Entry::Assistant { content: "回答".into(), usage: None, reasoning: None },
            Entry::ToolRequest { call_id: "c1".into(), name: "edit".into(), object: "./a.txt".into() },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: true,
                result: "- 旧行\n+ 新行".into(),
            },
            Entry::Error { text: "炸了".into() },
        ];
        let lines = render(&entries, &p, false, false);
        // User: accent bar; body styling is the markdown pipeline's job (default foreground)
        assert_eq!(lines[0].spans[0].content, "▌ ");
        assert_eq!(lines[0].spans[0].style.fg, Some(p.accent));
        // Row layout: user/assistant each take 2 rows (content + blank)
        let req = format!("{:?}", lines[4].spans[0].content);
        assert!(req.contains("edit") && req.contains("./a.txt"), "{req}");
        // Tool result@5 marker row; @6 deletion on red; @7 insertion on green
        let diff_del = format!("{:?}", lines[6].spans[0].content);
        assert!(diff_del.contains('-'), "{diff_del}");
        let diff_ins = format!("{:?}", lines[7].spans[0].content);
        assert!(diff_ins.contains('+'), "{diff_ins}");
        // Error@8: red
        assert_eq!(lines[8].style.fg, Some(Color::Red));
    }

    #[test]
    fn plain_view_folds_beyond_five_lines() {
        let p = Palette::default();
        let long = Entry::ToolResult {
            call_id: "c1".into(),
            name: "t".into(),
            ok: true,
            result: (1..=8).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n"),
        };
        let lines = render(&[long], &p, false, false);
        let texts: Vec<String> = lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect()).collect();
        let joined = texts.join("\n");
        assert!(joined.contains("line1") && joined.contains("line5"));
        assert!(!joined.contains("line6"));
        assert!(joined.contains("共 8 行"));
    }

    #[test]
    fn payload_round_trips_through_json() {
        let entries = vec![
            Entry::User { content: "你好\n世界".into() },
            Entry::Assistant {
                content: "回复".into(),
                reasoning: None,
                usage: Some(UsageSummary { total_tokens: 10, prompt_tokens: 8, cached_tokens: 2, completion_tokens: 2, reasoning_tokens: 0 }),
            },
            Entry::ToolRequest { call_id: "c1".into(), name: "edit".into(), object: "p".into() },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: false,
                result: "出错".into(),
            },
        ];
        for e in &entries {
            let (kind, payload) = e.to_payload();
            let back = Entry::from_payload(kind, &payload).expect("kind 必须可还原");
            assert_eq!(&back, e, "{kind} 往返失真");
        }
    }

    #[test]
    fn diff_view_uses_colored_backgrounds() {
        let p = Palette::default();
        let e = Entry::ToolResult {
            call_id: "c1".into(),
            name: "edit".into(),
            ok: true,
            result: "- 旧行\n+ 新行".into(),
        };
        let lines = render(&[e], &p, false, false);
        // Marker row + deletion + insertion
        assert_eq!(lines[1].style.bg, Some(Color::Rgb(255, 128, 128)));
        assert_eq!(lines[2].style.bg, Some(Color::Rgb(128, 200, 128)));
    }

    #[test]
    fn estimated_height_counts_wrapped_rows() {
        let lines = render(&[Entry::Assistant { content: "x".repeat(20), usage: None, reasoning: None }], &Palette::default(), false, false);
        // 20 cells wide, container 10 -> 2 rows; plus 1 blank row -> 3
        assert_eq!(estimated_height(&lines, 10), 3);
    }

    #[test]
    fn reasoning_hidden_by_default_shown_when_expanded() {
        let p = Palette::default();
        let entries = vec![Entry::Assistant {
            content: "答案".into(),
            usage: None,
            reasoning: Some("内心独白".into()),
        }];
        // Folded by default: reasoning fully hidden, content starts immediately
        let folded = render(&entries, &p, false, false);
        let folded_text = folded.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect::<String>();
        assert!(!folded_text.contains("内心独白"), "{folded_text}");
        // Expanded: the gray italic appears
        let open = render(&entries, &p, true, false);
        let open_text = open.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect::<String>();
        assert!(open_text.contains("内心独白"));
        let italic_line = open.iter().find(|l| l.spans.iter().any(|s| s.content == "内心独白")).unwrap();
        assert!(italic_line.spans[0].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn empty_reasoning_never_renders_even_expanded() {
        let p = Palette::default();
        let entries = vec![Entry::Assistant {
            content: "答案".into(),
            usage: None,
            reasoning: Some("  ".into()),
        }];
        let lines = render(&entries, &p, true, false);
        let text = lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect::<String>();
        // Content and blank row only, nothing extra
        assert!(text.contains("答案"));
        assert_eq!(lines.len(), 2, "正文 + 尾部空行: {text:?}");
    }

    #[test]
    fn reasoning_round_trips_through_payload() {
        let e = Entry::Assistant { content: "答".into(), usage: None, reasoning: Some("想了想".into()) };
        let (kind, payload) = e.to_payload();
        assert_eq!(Entry::from_payload(kind, &payload).unwrap(), e);
        // Old data lacks the reasoning field: reads as None without exploding
        let legacy = r#"{"content":"老消息","usage":null}"#;
        let back = Entry::from_payload("assistant", legacy).unwrap();
        assert_eq!(back, Entry::Assistant { content: "老消息".into(), usage: None, reasoning: None });
    }

    #[test]
    fn tool_collapse_limit_is_per_tool() {
        assert_eq!(collapse_limit("edit"), 14);
        assert_eq!(collapse_limit("mass_edit"), 14);
        assert_eq!(collapse_limit("未知工具"), 5);
    }

    #[test]
    fn plain_view_respects_per_tool_limit_and_expansion() {
        let p = Palette::default();
        let eight = (1..=8).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        // 8 rows: edit (threshold 14) does not fold; unknown tools (threshold 5) fold
        let edit_e = Entry::ToolResult { call_id: "c1".into(), name: "edit".into(), ok: true, result: eight.clone() };
        let other_e = Entry::ToolResult { call_id: "c".into(), name: "x".into(), ok: true, result: eight.clone() };
        let edit_lines = render(&[edit_e], &p, false, false);
        let edit_text = edit_lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect::<String>();
        assert!(edit_text.contains("line8") && !edit_text.contains("共"), "edit 8 行不折叠: {edit_text}");
        let other_lines = render(std::slice::from_ref(&other_e), &p, false, false);
        let other_text = other_lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect::<String>();
        assert!(other_text.contains("共 8 行"), "未知工具 8 行折叠: {other_text}");
        // Ctrl+O expand: folded output expands too
        let open_lines = render(&[other_e], &p, false, true);
        let open_text = open_lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect::<String>();
        assert!(open_text.contains("line8") && !open_text.contains("共"), "展开后无折叠: {open_text}");
    }

    #[test]
    fn live_thinking_line_disappears_once_content_starts() {
        let p = Palette::default();
        // Reasoning phase: the thinking line shows
        let during = render_with_live(&[], &p, false, false, Some("正在想"), false, None);
        assert_eq!(during.len(), 1);
        assert_eq!(during[0].spans[0].content, "thinking");
        assert!(during[0].style.add_modifier.contains(Modifier::ITALIC) || during[0].spans[0].style.add_modifier.contains(Modifier::ITALIC));
        // Content started: the thinking line withdraws, content takes its place
        let after = render_with_live(&[], &p, false, false, Some("正在想"), true, Some("你好"));
        assert_eq!(after.len(), 1, "只有正文: {after:?}");
        assert_eq!(after[0].spans[0].content, "你好");
    }

    #[test]
    fn tool_result_payload_stores_data_not_view() {
        // Persistence contract: the tool_result payload stores data only (call_id/name/ok/result),
        // the view is a derived UI concept synthesized at render time by ToolView::synthesize
        let e = Entry::ToolResult {
            call_id: "c9".into(),
            name: "edit".into(),
            ok: true,
            result: "- a\n+ b".into(),
        };
        let (_, payload) = e.to_payload();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v.get("view").is_none(), "payload 不该有 view 字段");
        assert_eq!(v.get("result").and_then(|r| r.as_str()), Some("- a\n+ b"));
        // After the round-trip the Diff is re-synthesized from data
        let back = Entry::from_payload("tool_result", &payload).unwrap();
        match ToolView::synthesize("edit", true, "- a\n+ b") {
            ToolView::Diff { deletions, insertions } => {
                assert_eq!(deletions, vec!["a"]);
                assert_eq!(insertions, vec!["b"]);
            }
            other => panic!("应合成出 Diff: {other:?}"),
        }
        assert_eq!(back, e);
    }
}
