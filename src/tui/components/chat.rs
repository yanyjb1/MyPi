//! Chat history: typed entries + rendering dispatched by kind.
//!
//! An early version stored messages as "prefix + body" strings in `Vec<String>`,
//! with renderers guessing the type via strip_prefix — a REPL-era relic,
//! now fully retired. History is an array of [`Entry`]; rendering dispatches on kind.
//!
//! Data flow: streaming writes only the in-memory in-progress slot; at TurnDone the
//! whole round's entries are persisted in one shot. Rendering only accepts `&[Entry]`,
//! so in-memory and DB-sourced history share one render path.
//!
//! Presentation rules (each kind owns its own look):
//! - **user** — a card: true-black background, white text, accent gutter;
//! - **assistant** — plain foreground, no background; reasoning in the muted gray;
//! - **tool** — two bordered cards (`+- … -+`), the call rendered above the result,
//!   both on the same true-black background, payload syntax-highlighted;
//! - **system** — a notice the emitter aligns (left or centered), e.g. compaction
//!   reports.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::entry::{Align, Entry, ToolView, UsageSummary};
use crate::tui::highlight;
use crate::tui::theme::Palette;

// Render entries into ratatui lines.
//
// The single render entry point: in-memory history and DB-resumed history
// both go through it, guaranteeing "reopened after persistence" looks identical to "just typed".
pub fn render(entries: &[Entry], p: &Palette, show_reasoning: bool, tools_expanded: bool) -> Vec<Line<'static>> {
    render_with_live(entries, p, show_reasoning, tools_expanded, 80, None, None, false, None)
}

// Render plus the streaming tail (in-flight reasoning/content/tool intent).
//
// `width` is the target column count: cards pad to it so their backgrounds
// form a solid block instead of stopping at the last glyph.
#[allow(clippy::too_many_arguments)]
pub fn render_with_live(
    entries: &[Entry],
    p: &Palette,
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
    live_reasoning: Option<&str>,
    live_intent: Option<&str>,
    reasoning_done: bool,
    streaming: Option<&str>,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            Entry::User { content } => out.extend(user_card(content, p, width)),
            Entry::Assistant { content, usage, reasoning } => {
                out.extend(assistant_block(content, reasoning.as_deref(), usage.as_ref(), p, show_reasoning))
            }
            Entry::ToolRequest { name, args, intent, .. } => {
                out.extend(tool_request_card(name, args, intent, p, width))
            }
            Entry::ToolResult { name, ok, result, .. } => {
                // The view is not persisted: synthesized from data at render time
                let view = ToolView::synthesize(name, *ok, result);
                out.extend(tool_result_card(name, *ok, &view, p, tools_expanded, width))
            }
            Entry::Error { text } => {
                for part in text.split('\n') {
                    out.push(Line::styled(part.to_string(), Style::new().fg(Color::Red)));
                }
            }
            // Emitter-aligned notice (model switches, compaction reports).
            Entry::System { text, align } => out.extend(system_block(text, *align, p, width)),
            // Name markers are metadata, not chat content: never a history row.
            Entry::Name { .. } => {}
        }
    }
    // ---- streaming tail ----
    // One live row, in priority order:
    //   1. a tool is running  -> the model's own intent line (what it is doing)
    //   2. reasoning only      -> "thinking"
    //   3. content has started -> withdrawn, the content itself takes the row
    if !reasoning_done
        && let Some(it) = live_intent.filter(|s| !s.trim().is_empty())
    {
        out.push(Line::styled(
            it.to_string(),
            Style::new().fg(p.muted).add_modifier(Modifier::ITALIC),
        ));
    } else if let Some(r) = live_reasoning
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

// ---------------------------------------------------------------------------
// card scaffolding
// ---------------------------------------------------------------------------

// Pad `line` out to `width` display cells with `fill`.
//
// Every card body goes through this: a background only covers the cells it
// actually paints, so an unpadded row would end mid-card.
fn pad_to(mut line: Line<'static>, width: usize, fill: Style) -> Line<'static> {
    let used: usize = line.spans.iter().map(|s| s.content.as_ref().width()).sum();
    if used < width {
        line.spans.push(Span::styled(" ".repeat(width - used), fill));
    }
    line
}

// Top edge: `+- <label> ` + dashes + `-+`. `edge` colors the frame.
fn card_top(left: &str, label: &str, edge: Style, fill: Style, width: usize) -> Line<'static> {
    let head = format!("{left} {label} ");
    let used = display_width(&head) + 2; // trailing `-+`
    let dashes = width.saturating_sub(used);
    Line::from(vec![
        Span::styled(head, edge),
        Span::styled("-".repeat(dashes), edge),
        Span::styled("-+", edge),
        Span::styled(" ".to_string(), fill),
    ])
}

// Bottom edge: `+-` + dashes + `-+`.
fn card_bottom(edge: Style, fill: Style, width: usize) -> Line<'static> {
    let dashes = width.saturating_sub(4);
    Line::from(vec![
        Span::styled("+-", edge),
        Span::styled("-".repeat(dashes), edge),
        Span::styled("-+", edge),
        Span::styled(" ".to_string(), fill),
    ])
}

// One body row: `| ` + content + padding + ` |`.
fn card_row(content: Line<'static>, edge: Style, body_bg: Style, width: usize) -> Line<'static> {
    let inner = width.saturating_sub(4);
    let mut spans = vec![Span::styled("| ", edge)];
    // `Line::styled(x, s)` puts `s` on the **line**, not on its spans, so a
    // caller that styles a whole row (the diff's red/green) would otherwise
    // lose that color the moment we take the spans apart. Fold it in first.
    let line_style = content.style;
    let content: Vec<Span<'static>> = content
        .spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, line_style.patch(sp.style)))
        .collect();
    // Reserve the right border before padding the content.
    let used: usize = content.iter().map(|s| s.content.as_ref().width()).sum();
    let clipped: Vec<Span<'static>> = if used > inner {
        clip_spans(content, inner)
    } else {
        content
    };
    let used: usize = clipped.iter().map(|s| s.content.as_ref().width()).sum();
    spans.extend(clipped);
    if used < inner {
        spans.push(Span::styled(" ".repeat(inner - used), body_bg));
    }
    spans.push(Span::styled(" |", edge));
    Line::from(spans)
}

// Cut spans at `max` display cells (keeps a card row inside its borders).
fn clip_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for sp in spans {
        let w = sp.content.as_ref().width();
        if used + w <= max {
            used += w;
            out.push(sp);
        } else {
            let keep = max.saturating_sub(used);
            if keep > 0 {
                let (cut, _) = crate::tui::text::take_width(&sp.content, keep);
                out.push(Span::styled(cut, sp.style));
            }
            break;
        }
    }
    out
}

fn display_width(s: &str) -> usize {
    s.width()
}

// ---------------------------------------------------------------------------
// per-kind blocks
// ---------------------------------------------------------------------------

// User message: a card on the true-black background, white text.
//
// The black is the palette's truecolor black (`0,0,0`), never the ANSI
// indexed black — indexed black is what terminals map to gray, which is
// exactly the look we are avoiding here.
fn user_card(content: &str, p: &Palette, width: usize) -> Vec<Line<'static>> {
    let bg = Style::new().bg(p.black);
    let mut out = Vec::new();
    for line in super::markdown::render_markdown(content, p) {
        // Force the card's own foreground/background: markdown may have
        // decided on a color for a code span, but a user message is
        // uniformly black-on-… white-on-black.
        let mut spans = vec![Span::styled(
            "▌ ",
            Style::new().bg(p.black).fg(p.accent),
        )];
        for sp in line.spans {
            spans.push(Span::styled(sp.content, Style::new().bg(p.black).fg(Color::White)));
        }
        out.push(pad_to(Line::from(spans), width, bg));
    }
    if !out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

// Model reply: plain foreground, no background. Reasoning (when shown) is muted.
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
    // Reasoning: the thinking chain, shown by default. An earlier revision hid
    // it unless Ctrl+T was pressed; the toggle now only suppresses it.
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

// System notice: the emitter chose the alignment, we only place it.
fn system_block(text: &str, align: Align, p: &Palette, width: usize) -> Vec<Line<'static>> {
    let style = Style::new().fg(p.muted);
    let mut out = Vec::new();
    for part in text.split('\n') {
        let w = display_width(part);
        match align {
            Align::Left => out.push(Line::styled(part.to_string(), style)),
            Align::Center => {
                let pad = width.saturating_sub(w) / 2;
                out.push(Line::from(vec![
                    Span::styled(" ".repeat(pad), style),
                    Span::styled(part.to_string(), style),
                ]));
            }
        }
    }
    out.push(Line::from(""));
    out
}

// ---------------------------------------------------------------------------
// tool cards
// ---------------------------------------------------------------------------

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

// Tool call card: the arguments, syntax-highlighted, on the black card.
//
// The model's `intent` heads the card: it is the one-line "what I am about
// to do" written for a human, and it is also what the live slot shows while
// the tool blocks the turn.
fn tool_request_card(name: &str, args: &str, intent: &str, p: &Palette, width: usize) -> Vec<Line<'static>> {
    let edge = Style::new().fg(p.accent);
    let bg = Style::new().bg(p.black);
    let label = if intent.trim().is_empty() {
        // Older entries (and models that skip the field) have no intent; a
        // neutral label beats an empty parenthesis.
        format!("{name} (in)")
    } else {
        format!("{name} (in) · {intent}")
    };
    let mut out = vec![card_top("+-", &label, edge, bg, width)];
    for line in payload_lines(name, args, p) {
        out.push(card_row(line, edge, bg, width));
    }
    out.push(card_bottom(edge, bg, width));
    out
}

// Tool result card: `✓`/`✗` in the label, body drawn per the tool's view.
// `expanded` is the global switch (Ctrl+O): true ignores thresholds and expands everything.
fn tool_result_card(
    name: &str,
    ok: bool,
    view: &ToolView,
    p: &Palette,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mark = if ok { "✓" } else { "✗" };
    let edge = Style::new().fg(if ok { Color::Green } else { Color::Red });
    let bg = Style::new().bg(p.black);
    let mut out = vec![card_top("+-", &format!("{mark} {name} (out)"), edge, bg, width)];

    match view {
        ToolView::Plain { text } => {
            let lines: Vec<&str> = text.lines().collect();
            let limit = collapse_limit(name);
            let fold = !expanded && lines.len() > limit;
            let shown = if fold { &lines[..limit] } else { &lines[..] };
            // Language comes from the tool itself (bash speaks shell); the
            // generic case has no hint and degrades to plain text.
            let lang = highlight::language_for_tool(name);
            for l in shown {
                for hl in highlight::highlight(l, lang) {
                    out.push(card_row(hl, edge, bg, width));
                }
            }
            if fold {
                out.push(card_row(
                    Line::styled(format!("… 共 {} 行", lines.len()), Style::new().fg(p.muted)),
                    edge,
                    bg,
                    width,
                ));
            }
        }
        ToolView::Diff { deletions, insertions } => {
            // Deletions above, insertions below: red and green backgrounds
            // (vscode style). Still routed through `card_row`, so the diff sits
            // inside the same `| … |` frame as every other card body.
            for d in deletions {
                out.push(card_row(
                    Line::styled(
                        format!("- {d}"),
                        Style::new().fg(Color::Black).bg(Color::Rgb(255, 128, 128)),
                    ),
                    edge,
                    bg,
                    width,
                ));
            }
            for i in insertions {
                out.push(card_row(
                    Line::styled(
                        format!("+ {i}"),
                        Style::new().fg(Color::Black).bg(Color::Rgb(128, 200, 128)),
                    ),
                    edge,
                    bg,
                    width,
                ));
            }
        }
    }
    out.push(card_bottom(edge, bg, width));
    out
}

// The call's arguments as highlighted code.
//
// `bash` payloads are shell; edit-family payloads are file content, so the
// target path's extension picks the syntax. Anything unparsable still renders
// verbatim — a card must never eat the model's actual arguments.
fn payload_lines(name: &str, args: &str, p: &Palette) -> Vec<Line<'static>> {
    let _ = p;
    let v: Option<serde_json::Value> = serde_json::from_str(args).ok();
    if let Some(v) = &v {
        // bash: show the command itself, not a JSON blob.
        if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
            let mut out = Vec::new();
            for l in cmd.lines() {
                out.extend(highlight::highlight(l, Some("sh")));
            }
            return out;
        }
        // edit / mass_edit: highlight by the target file's language.
        if let Some(path) = v.get("path").and_then(|c| c.as_str()) {
            let lang = highlight::language_for_path(path);
            let mut out = vec![Line::styled(
                format!("path: {path}"),
                Style::new().fg(Color::White),
            )];
            if let Some(old) = v.get("old").and_then(|c| c.as_str()) {
                out.push(Line::styled("- old:", Style::new().fg(Color::Rgb(255, 128, 128))));
                for l in old.lines() {
                    out.extend(highlight::highlight(l, lang));
                }
            }
            if let Some(new) = v.get("new").and_then(|c| c.as_str()) {
                out.push(Line::styled("+ new:", Style::new().fg(Color::Rgb(128, 200, 128))));
                for l in new.lines() {
                    out.extend(highlight::highlight(l, lang));
                }
            }
            return out;
        }
        // Fallback: pretty JSON, so the arguments stay readable.
        let pretty = serde_json::to_string_pretty(v).unwrap_or_else(|_| args.to_string());
        let mut out = Vec::new();
        for l in pretty.lines() {
            out.extend(highlight::highlight(l, Some("json")));
        }
        return out;
    }
    let mut out = Vec::new();
    for l in args.lines() {
        out.extend(highlight::highlight(l, highlight::language_for_tool(name)));
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

    fn text_of(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn entries_render_by_kind_not_by_prefix() {
        let p = Palette::default();
        let entries = vec![
            Entry::User { content: "问题".into() },
            Entry::Assistant { content: "回答".into(), usage: None, reasoning: None },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "edit".into(),
                args: r#"{"path":"./a.txt"}"#.into(),
                intent: "改文件".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: true,
                result: "- 旧行\n+ 新行".into(),
            },
            Entry::Error { text: "炸了".into() },
        ];
        let lines = render(&entries, &p, false, false);
        let all = text_of(&lines);
        // User card: accent gutter on a true-black background
        assert_eq!(lines[0].spans[0].content, "▌ ");
        assert_eq!(lines[0].spans[0].style.fg, Some(p.accent));
        assert_eq!(lines[0].spans[0].style.bg, Some(Color::Rgb(0, 0, 0)));
        // Both tool cards exist, labelled in/out
        assert!(all.contains("(in)"), "{all}");
        assert!(all.contains("(out)"), "{all}");
        // The call card carries the path…
        assert!(all.contains("./a.txt"), "{all}");
        // …and the diff body keeps its red/green rows (inside the card frame,
        // so the color sits on the span rather than the whole line)
        let bgs: Vec<_> = lines.iter().flat_map(|l| l.spans.iter()).map(|s| s.style.bg).collect();
        assert!(bgs.contains(&Some(Color::Rgb(255, 128, 128))), "{all}");
        assert!(bgs.contains(&Some(Color::Rgb(128, 200, 128))), "{all}");
        // Error stays red
        assert_eq!(lines.last().unwrap().style.fg, Some(Color::Red));
    }

    #[test]
    fn user_card_is_white_on_true_black() {
        let p = Palette::default();
        let lines = user_card("你好", &p, 20);
        // Every body span: white on truecolor black (never the indexed black)
        for l in lines.iter().take(1) {
            for (i, sp) in l.spans.iter().enumerate() {
                assert_eq!(sp.style.bg, Some(Color::Rgb(0, 0, 0)), "用户消息必须是真彩黑底");
                // span 0 is the accent gutter, not message text
                if i > 0 && !sp.content.trim().is_empty() {
                    assert_eq!(sp.style.fg, Some(Color::White), "用户消息必须是白字");
                }
            }
        }
    }

    #[test]
    fn tool_cards_are_bordered() {
        let p = Palette::default();
        let call = tool_request_card("bash", r#"{"command":"ls -la"}"#, "看目录", &p, 30);
        // Top edge: `+- bash (in) · 看目录 ----`
        assert!(text_of(&call[..1]).starts_with("+- bash (in)"), "{}", text_of(&call[..1]));
        // Body row: pipes with a space inside both ends
        let body = text_of(&call[1..2]);
        assert!(body.starts_with("| "), "{body:?}");
        assert!(body.trim_end().ends_with("|"), "{body:?}");
        // Bottom edge
        assert!(text_of(&call[call.len() - 1..]).trim_end().starts_with("+-"), "卡片必须有底边");
        // Every row is exactly `width` cells wide, so the black block is solid
        for l in &call {
            let w: usize = l.spans.iter().map(|s| s.content.as_ref().width()).sum();
            assert!(w >= 30, "卡片行必须补满宽度: {w}");
        }
    }

    #[test]
    fn bash_payload_prettifies_to_shell_not_json() {
        let p = Palette::default();
        let lines = payload_lines("bash", r#"{"command":"cargo build --release"}"#, &p);
        let all = text_of(&lines);
        assert!(all.contains("cargo build --release"), "{all}");
        assert!(!all.contains('{'), "不该把 JSON 外壳渲出来: {all}");
        assert!(!all.contains("command"), "{all}");
    }

    #[test]
    fn payload_highlight_uses_the_paths_language() {
        let p = Palette::default();
        let lines = payload_lines("edit", r#"{"path":"main.rs","old":"let a","new":"let b"}"#, &p);
        let all = text_of(&lines);
        assert!(all.contains("main.rs") && all.contains("let a") && all.contains("let b"), "{all}");
    }

    #[test]
    fn system_notice_honours_its_alignment() {
        let p = Palette::default();
        let left = system_block("切到 X", Align::Left, &p, 20);
        assert!(text_of(&left).starts_with("切到 X"));
        let mid = system_block("压缩完成", Align::Center, &p, 20);
        let row = text_of(&mid[..1]);
        let pad = row.chars().take_while(|c| *c == ' ').count();
        // "压缩完成" is 4 CJK glyphs = 8 cells, so (20 - 8) / 2
        assert_eq!(pad, (20 - 8) / 2, "居中必须左右留白: {row:?}");
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
        let joined = text_of(&lines);
        assert!(joined.contains("line1") && joined.contains("line5"));
        assert!(!joined.contains("line6"), "{joined}");
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
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "edit".into(),
                args: r#"{"path":"p"}"#.into(),
                intent: "改一下".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: false,
                result: "出错".into(),
            },
            Entry::System { text: "已切换模型".into(), align: Align::Center },
        ];
        for e in &entries {
            let (kind, payload) = e.to_payload();
            let back = Entry::from_payload(kind, &payload).expect("kind 必须可还原");
            assert_eq!(&back, e, "{kind} 往返失真");
        }
    }

    #[test]
    fn legacy_tool_request_without_args_still_loads() {
        // Pre-refactor rows stored only `object`; they must not break resume.
        let old = r#"{"call_id":"c1","name":"bash","object":""}"#;
        let back = Entry::from_payload("tool_request", old).unwrap();
        match back {
            Entry::ToolRequest { args, intent, name, .. } => {
                assert_eq!(args, "");
                assert_eq!(intent, "");
                assert_eq!(name, "bash");
            }
            other => panic!("{other:?}"),
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
        // The colored rows live inside the card frame, so the red/green is the
        // inner span's style, not the row's.
        let all: Vec<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.style.bg)
            .collect();
        assert!(all.contains(&Some(Color::Rgb(255, 128, 128))), "删除行必须红底");
        assert!(all.contains(&Some(Color::Rgb(128, 200, 128))), "插入行必须绿底");
    }

    #[test]
    fn estimated_height_counts_wrapped_rows() {
        let lines = render(&[Entry::Assistant { content: "x".repeat(20), usage: None, reasoning: None }], &Palette::default(), false, false);
        // 20 cells wide, container 10 -> 2 rows; plus 1 blank row -> 3
        assert_eq!(estimated_height(&lines, 10), 3);
    }

    #[test]
    fn reasoning_shown_by_default_hidden_when_folded() {
        let p = Palette::default();
        let entries = vec![Entry::Assistant {
            content: "答案".into(),
            usage: None,
            reasoning: Some("内心独白".into()),
        }];
        // Default view: reasoning is visible, in the muted gray
        let open = render(&entries, &p, true, false);
        let open_text = text_of(&open);
        assert!(open_text.contains("内心独白"), "{open_text}");
        let r_line = open
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content == "内心独白"))
            .unwrap();
        assert_eq!(r_line.spans[0].style.fg, Some(p.muted));
        assert!(r_line.spans[0].style.add_modifier.contains(Modifier::ITALIC));
        // Folded: gone
        let folded = render(&entries, &p, false, false);
        assert!(!text_of(&folded).contains("内心独白"));
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
        let text = text_of(&lines);
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
    fn inline_code_stays_on_one_line() {
        let p = Palette::default();
        let lines = super::super::markdown::render_markdown(
            "已用 `edit` 工具将 `main.rs` 中的 `hi` 改为 `hello`。",
            &p,
        );
        assert_eq!(lines.len(), 1, "行内代码不该换行: {:?}", text_of(&lines));
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
        let edit_text = text_of(&render(&[edit_e], &p, false, false));
        assert!(edit_text.contains("line8") && !edit_text.contains("共"), "edit 8 行不折叠: {edit_text}");
        let other_text = text_of(&render(std::slice::from_ref(&other_e), &p, false, false));
        assert!(other_text.contains("共 8 行"), "未知工具 8 行折叠: {other_text}");
        // Ctrl+O expand: folded output expands too
        let open_text = text_of(&render(&[other_e], &p, false, true));
        assert!(open_text.contains("line8") && !open_text.contains("共"), "展开后无折叠: {open_text}");
    }

    #[test]
    fn live_row_shows_intent_then_thinking_then_content() {
        let p = Palette::default();
        // A tool is running: the model's intent takes the live row.
        let during_tool = render_with_live(&[], &p, false, false, 40, Some("想"), Some("正在编译"), false, None);
        assert_eq!(during_tool.len(), 1);
        assert_eq!(during_tool[0].spans[0].content, "正在编译");
        // No tool, but reasoning: the generic label.
        let thinking = render_with_live(&[], &p, false, false, 40, Some("想"), None, false, None);
        assert_eq!(thinking[0].spans[0].content, "thinking");
        // Content started: the live row is withdrawn, content takes its place.
        let after = render_with_live(&[], &p, false, false, 40, Some("想"), Some("正在编译"), true, Some("你好"));
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
