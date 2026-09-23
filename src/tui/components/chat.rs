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
#[cfg(test)]
use ratatui::widgets::Paragraph;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::entry::{Align, Entry, ToolView, UsageSummary};
use crate::server::events::LiveActivity;
use crate::tui::highlight;
use crate::tui::theme::Palette;

// Render entries into ratatui lines.
//
// The single render entry point: in-memory history and DB-resumed history
// both go through it, guaranteeing "reopened after persistence" looks identical to "just typed".
pub fn render(entries: &[Entry], p: &Palette, show_reasoning: bool, tools_expanded: bool) -> Vec<Line<'static>> {
    render_with_live(entries, p, show_reasoning, tools_expanded, 80, &LiveActivity::Idle, None)
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
    live: &LiveActivity,
    streaming: Option<&str>,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    // A tool result immediately following its request is the same exchange:
    // render them as one stacked card instead of two separate ones.
    let mut i = 0;
    while i < entries.len() {
        // One blank row before every node but the first: uniform separation,
        // so a user message and the reply under it never fuse into one block.
        if !out.is_empty() {
            out.push(section_gap());
        }
        let e = &entries[i];
        if let (Entry::ToolRequest { call_id, name, args, .. }, Some(Entry::ToolResult { call_id: rid, name: rname, ok, result, .. })) =
            (e, entries.get(i + 1))
            && call_id == rid
            && name == rname
        {
            // `read` has no result card: it is side-effect-free and the
            // request card (the path) already tells the whole story, so the
            // file contents would just be the same thing twice.
            if result_card_visible(name) {
                out.extend(tool_exchange(name, args, *ok, result, p, tools_expanded, width));
            } else {
                out.extend(tool_request_card(name, args, p, width));
            }
            i += 2;
            continue;
        }
        match e {
            Entry::User { content } => out.extend(user_card(content, p, width)),
            Entry::Assistant { content, usage, reasoning } => {
                out.extend(assistant_block(content, reasoning.as_deref(), usage.as_ref(), p, show_reasoning))
            }
            Entry::ToolRequest { name, args, .. } => {
                out.extend(tool_request_card(name, args, p, width))
            }
            Entry::ToolResult { name, ok, result, .. } => {
                out.extend(tool_result_card(name, *ok, result, p, tools_expanded, width))
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
        i += 1;
    }
    // ---- streaming tail: the one live row at the bottom ----
    //
    // It is a **system notice drawn last**, i.e. bottom-most in the history
    // area. Three states, driven by the session (not guessed here):
    //   Thinking      — the server has started emitting reasoning
    //   Tool { intent } — a tool is running; the model's own explanation, or
    //                   a bare label for tools that need none (read/edit)
    //   Idle          — nothing pending (arriving content occupies the row)
    match live {
        LiveActivity::Thinking => {
            out.push(Line::styled(
                "thinking",
                Style::new().fg(p.muted).add_modifier(Modifier::ITALIC),
            ));
        }
        LiveActivity::Tool { intent } => {
            let label = if intent.trim().is_empty() { "working" } else { intent.as_str() };
            out.push(Line::styled(
                label.to_string(),
                Style::new().fg(p.muted).add_modifier(Modifier::ITALIC),
            ));
        }
        LiveActivity::Idle => {}
    }
    // In-flight content: gray italic would be wrong (it is the final answer); default style, appended per delta
    if let Some(t) = streaming
        && !t.is_empty()
    {
        out.extend(super::markdown::render_markdown(t, p));
    }
    out
}

// The blank row between two transcript nodes.
//
// Every node is separated from its neighbours by exactly one of these —
// uniform rather than each kind remembering to pad itself, so two adjacent
// nodes can never look like one.
fn section_gap() -> Line<'static> {
    Line::from("")
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

// Give every cell of a card row the card's background.
//
// A background only covers the cells it paints, and painting it on the
// padding alone left the text sitting on the terminal default — so a card
// looked black in the gaps and *not* black behind the words. Every span
// (frame included) goes through here.
fn on_bg(spans: Vec<Span<'static>>, bg: Style) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, bg.patch(sp.style)))
        .collect()
}

// A card's horizontal edge, exactly `width` cells:
//
//     +- - -------…------- -+
//     └┬┘└┬┘        └┬┘└┬┘
//      │  │          │  └── right break: one space, one dash, corner
//      │  └──────────────  left break: one dash, one space
//      └─────────────────── corner
//
// The two single-dash **breaks** exist for ligature-capable fonts: Sarasa and
// friends fuse a consecutive run of dashes into one glyph and draw it narrower
// than the cells we reserved, so the frame stopped short of the body. Breaking
// the run at both ends keeps the corners from touching the middle, which is
// enough to defeat the fusion — while the long middle stretch still reads as
// one continuous rule (the earlier attempt to space *every* dash looked like a
// dotted line and was rejected).
//
// Both ends give up one cell to a break. Degenerate widths fall back to a plain
// run so the row can never exceed the budget.
fn card_edge(edge: Style, fill: Style, width: usize) -> Line<'static> {
    // `+-` + `-` + ` ` + mid + ` ` + `-` + `-+` = 7 cells of frame/scaffolding.
    if width < 8 {
        let dashes = width.saturating_sub(4);
        return Line::from(on_bg(
            vec![
                Span::styled("+-", edge),
                Span::styled("-".repeat(dashes), edge),
                Span::styled("-+", edge),
            ],
            fill,
        ));
    }
    let mid = width - 7;
    Line::from(on_bg(
        vec![
            Span::styled("+-", edge),
            Span::styled("- ", edge),
            Span::styled("-".repeat(mid), edge),
            Span::styled(" -+", edge),
        ],
        fill,
    ))
}

// One body row: `| ` + content + padding + ` |`.
fn card_row(content: Line<'static>, edge: Style, body_bg: Style, width: usize) -> Line<'static> {
    let inner = width.saturating_sub(4);
    // Defensive: never let stray escape bytes reach the terminal from inside a
    // card. The tools strip their own output, but file contents and rows read
    // back from an older DB can still carry them.
    let clean_style = content.style;
    let content = Line::from(
        content
            .spans
            .into_iter()
            .map(|sp| Span::styled(crate::tui::text::strip_ansi(&sp.content), sp.style))
            .collect::<Vec<_>>(),
    )
    .style(clean_style);
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
    // Frame, text and padding all sit on the card's black.
    Line::from(on_bg(spans, body_bg))
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
    // Breathing room: a blank row above and below, still on the accent gutter
    // and the true-black background, so the card reads as one solid block.
    out.push(pad_to(Line::from(Span::styled("▌ ", Style::new().bg(p.black).fg(p.accent))), width, bg));
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
    out.push(pad_to(Line::from(Span::styled("▌ ", Style::new().bg(p.black).fg(p.accent))), width, bg));
    out
}

// (the trailing blank row between nodes is `section_gap`, added by the caller)

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
    }
    out.extend(super::markdown::render_markdown(content, p));
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
    out
}

// ---------------------------------------------------------------------------
// tool cards
// ---------------------------------------------------------------------------

// Whether a result deserves its own card.
//
// Reading is not a change: `read`'s result is the file it just showed, and
// the request card already names the path, so a result card would repeat it.
// Every other tool did something worth confirming.
fn result_card_visible(name: &str) -> bool {
    name != "read"
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

// Two cards glued into one: the call's content, the seam, the result's content.
//
// A tool call and its result are one exchange, not two messages — drawing
// them as two separate boxes (each with its own top *and* bottom edge) made
// a single `ls` cost six rows. Here the middle edge is shared, and the
// result's edges take the success/failure color so the outcome still reads
// at a glance. No type labels: the content says what it is.
fn tool_exchange(
    name: &str,
    args: &str,
    ok: bool,
    result: &str,
    p: &Palette,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let edge = Style::new().fg(p.accent);
    let out_edge = Style::new().fg(if ok { Color::Green } else { Color::Red });
    let bg = Style::new().bg(p.black);

    let mut out = vec![card_edge(edge, bg, width)];
    for line in payload_lines(name, args, p) {
        out.push(card_row(line, edge, bg, width));
    }
    // The seam: `+-` on the left and `-+` on the right, one shared edge.
    out.push(card_edge(edge, bg, width));
    for line in result_lines(name, ok, result, p, expanded) {
        out.push(card_row(line, out_edge, bg, width));
    }
    out.push(card_edge(out_edge, bg, width));
    out
}

// Tool call card: the arguments only, syntax-highlighted, on the black card.
//
// Used for a request that has no result yet (interrupt mid-call). The
// labelable form is [`tool_exchange`].
fn tool_request_card(name: &str, args: &str, p: &Palette, width: usize) -> Vec<Line<'static>> {
    let edge = Style::new().fg(p.accent);
    let bg = Style::new().bg(p.black);
    let mut out = vec![card_edge(edge, bg, width)];
    for line in payload_lines(name, args, p) {
        out.push(card_row(line, edge, bg, width));
    }
    out.push(card_edge(edge, bg, width));
    out
}

// Tool result card: `✓`/`✗` in the label, body drawn per the tool's view.
// `expanded` is the global switch (Ctrl+O): true ignores thresholds and expands everything.
fn tool_result_card(
    name: &str,
    ok: bool,
    result: &str,
    p: &Palette,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    // An unpaired result (interrupted call): colour tells the outcome, the
    // content tells the rest. No labels, same as every other card.
    let edge = Style::new().fg(if ok { Color::Green } else { Color::Red });
    let bg = Style::new().bg(p.black);
    let mut out = vec![card_edge(edge, bg, width)];
    for line in result_lines(name, ok, result, p, expanded) {
        out.push(card_row(line, edge, bg, width));
    }
    out.push(card_edge(edge, bg, width));
    out
}

// The body rows of a tool's result — shared by the standalone card and the
// stacked exchange, so both fold/diff/placeholder identically.
fn result_lines(
    name: &str,
    ok: bool,
    result: &str,
    p: &Palette,
    expanded: bool,
) -> Vec<Line<'static>> {
    let view = ToolView::synthesize(name, ok, result);
    let mut out = Vec::new();
    match &view {
        ToolView::Plain { text } => {
            // A tool that ran and returned nothing still **happened**: it cost
            // a round trip and may have changed the world (a silent `mkdir`,
            // a `git commit` that prints on success only). Show a placeholder
            // so the model — and the user — can see it was not skipped.
            if text.trim().is_empty() {
                // `read` has its own row even when the payload is the file.
                out.push(Line::styled("(no output)", Style::new().fg(p.muted)));
                return out;
            }
            let lines: Vec<&str> = text.lines().collect();
            let limit = collapse_limit(name);
            let fold = !expanded && lines.len() > limit;
            let shown = if fold { &lines[..limit] } else { &lines[..] };
            // Language comes from the tool itself (bash speaks shell); the
            // generic case has no hint and degrades to plain text.
            let lang = highlight::language_for_tool(name);
            for l in shown {
                for hl in highlight::highlight(l, lang) {
                    out.push(hl);
                }
            }
            if fold {
                out.push(Line::styled(
                    format!("… 共 {} 行", lines.len()),
                    Style::new().fg(p.muted),
                ));
            }
        }
        ToolView::Diff { deletions, insertions } => {
            // Deletions above, insertions below: red and green backgrounds
            // (vscode style). Kept as row styles; `card_row` folds them into
            // the spans so they survive the frame.
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
        // User card opens with a blank accent row on a true-black background
        assert_eq!(lines[0].spans[0].content, "▌ ");
        assert_eq!(lines[0].spans[0].style.fg, Some(p.accent));
        assert_eq!(lines[0].spans[0].style.bg, Some(Color::Rgb(0, 0, 0)));
        // The matched pair renders as one stacked card: no in/out labels
        assert!(!all.contains("(in)") && !all.contains("(out)"), "{all}");
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
    fn user_card_has_blank_rows_above_and_below() {
        let p = Palette::default();
        let lines = user_card("你好", &p, 20);
        assert_eq!(lines.len(), 3, "空行 + 正文 + 空行");
        for i in [0, 2] {
            let t = text_of(std::slice::from_ref(&lines[i]));
            assert_eq!(t.trim(), "▌", "空行只有 accent 竖线: {t:?}");
            // …and the black background still covers the whole row
            for sp in &lines[i].spans {
                assert_eq!(sp.style.bg, Some(Color::Rgb(0, 0, 0)), "空行也必须带真彩黑底");
            }
        }
    }

    #[test]
    fn user_card_is_white_on_true_black() {
        let p = Palette::default();
        let lines = user_card("你好", &p, 20);
        // Every body span: white on truecolor black (never the indexed black)
        for l in lines.iter().take(2).skip(1) {
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
        let call = tool_request_card("bash", r#"{"command":"ls -la"}"#, &p, 30);
        // Top edge: bare frame, no tool name / in-out labels
        assert!(text_of(&call[..1]).starts_with("+-"), "{}", text_of(&call[..1]));
        assert!(!text_of(&call).contains("(in)"), "不该再声明卡片类型");
        // Body row: pipes with a space inside both ends
        let body = text_of(&call[1..2]);
        assert!(body.starts_with("| "), "{body:?}");
        assert!(body.trim_end().ends_with("|"), "{body:?}");
        // Bottom edge
        assert!(text_of(&call[call.len() - 1..]).trim_end().starts_with("+-"), "卡片必须有底边");
        // Every row is **exactly** `width` cells: wider and the hard-wrap pass
        // would push the excess onto a line of its own (a stray black square).
        for l in &call {
            let w: usize = l.spans.iter().map(|s| s.content.as_ref().width()).sum();
            assert_eq!(w, 30, "卡片行的宽度必须恰好等于目标宽度: {w}");
        }
    }

    #[test]
    fn card_edges_break_the_dash_run_at_both_ends() {
        // The agreed shape (option F): symmetric breaks so a ligating font
        // cannot fuse the run and draw the frame short.
        let p = Palette::default();
        let rows = tool_exchange(
            "bash", r#"{"command":"ls"}"#, true, "out", &p, false, 40,
        );
        let edges: Vec<String> = rows
            .iter()
            .map(|l| text_of(std::slice::from_ref(l)))
            .filter(|t| t.starts_with("+-"))
            .collect();
        assert_eq!(edges.len(), 3, "顶边/中缝/底边: {edges:?}");
        for t in &edges {
            // Confirmed shape: `+-- <one long run> -+`. Each end gives up one
            // cell to a break (the left shows two dashes because the corner
            // itself is a dash).
            assert!(t.starts_with("+-- "), "左断口必须是 `+-- `: {t:?}");
            assert!(t.ends_with(" -+"), "右断口必须是 ` -+`: {t:?}");
            // …and the middle is still one continuous stretch.
            let inner = &t[4..t.len() - 4];
            assert!(inner.len() > 10 && !inner.contains(' '), "中段必须连续: {t:?}");
        }
    }

    #[test]
    fn no_card_row_exceeds_the_target_width() {
        // Regression for the phantom black square: the edges used to emit one
        // cell too many, which wrapped into a row of its own.
        let p = Palette::default();
        for width in [20usize, 40, 80, 121] {
            let rows = tool_exchange(
                "bash",
                r#"{"command":"sleep 5"}"#,
                true,
                "done",
                &p,
                false,
                width,
            );
            for l in &rows {
                let w: usize = l.spans.iter().map(|s| s.content.as_ref().width()).sum();
                assert_eq!(w, width, "({width}) 行宽必须恰好: {w} -> {:?}", text_of(std::slice::from_ref(l)));
            }
        }
    }

    #[test]
    fn every_cell_of_a_card_carries_the_black_background() {
        // The reported bug: the card looked black only in the gaps, because
        // the text spans carried a foreground but no background, and stray
        // ANSI from the command reset it. Every span of every card row —
        // frame, text, padding — must paint the truecolor black.
        let p = Palette::default();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"ls --color=always"}"#.into(),
                intent: "列目录".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                // A result that (from an older DB, say) still holds ANSI.
                result: "\u{1b}[01;34mdir\u{1b}[0m\nplain\n".into(),
            },
        ];
        let lines = render(&entries, &p, false, false);
        let black = Some(Color::Rgb(0, 0, 0));
        for l in &lines {
            // Only card rows (those with a frame) are checked; gaps have no spans.
            if l.spans.iter().any(|s| s.content.contains('|') || s.content.contains('+')) {
                for sp in &l.spans {
                    assert_eq!(sp.style.bg, black, "卡片每一格都必须是真彩黑底: {:?}", sp.content);
                }
            }
        }
        // Escape bytes must not survive into the drawn text.
        let all = text_of(&lines);
        assert!(!all.contains('\u{1b}'), "结果里的 ANSI 必须被剥离: {all:?}");
        assert!(all.contains("dir") && all.contains("plain"), "{all}");
    }

    #[test]
    fn backend_paints_black_across_the_whole_card_row() {
        // End-to-end through ratatui's own renderer: draw a card into a
        // TestBackend and read back the **composed cell styles**. This is the
        // check that actually proves no hole in the background, because it
        // goes through the same path the terminal does.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let p = Palette::default();
        let rows = tool_exchange(
            "bash",
            r#"{"command":"ls --color=always"}"#,
            true,
            "\u{1b}[01;34mdir\u{1b}[0m\nplain",
            &p,
            false,
            40,
        );
        let mut term = Terminal::new(TestBackend::new(40, rows.len() as u16)).unwrap();
        term.draw(|f| {
            f.render_widget(Paragraph::new(rows.clone()), f.area());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        // Every cell of the card's body rows must be black-backed.
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                assert_eq!(
                    cell.bg,
                    ratatui::style::Color::Rgb(0, 0, 0),
                    "卡片 ({x},{y}) 的格子没有黑底: {:?}",
                    cell.symbol()
                );
            }
        }
    }

    #[test]
    fn hard_wrap_adds_no_row_for_a_real_transcript() {
        // The phantom-cell bug: an over-wide row was split by `hard_wrap` into
        // an extra row holding only the overflow (a lone black cell). Prove
        // the wrapped row count equals the input count for a realistic
        // transcript, at several widths.
        for width in [30usize, 40, 80, 120] {
            let entries = vec![
                Entry::User { content: "跑一下 sleep 5 看看".into() },
                Entry::ToolRequest {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    args: r#"{"command":"sleep 5 && echo done"}"#.into(),
                    intent: "等五秒".into(),
                },
                Entry::ToolResult {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    ok: true,
                    result: "done".into(),
                },
            ];
            let lines = render_with_live(&entries, &Palette::default(), false, false, width, &LiveActivity::Idle, None);
            let before = lines.len();
            let wrapped = crate::tui::view::hard_wrap_for_test(&lines, width);
            assert_eq!(
                wrapped.len(),
                before,
                "({width}) 不该有任何行被硬换行切出额外的行"
            );
            // And the blank separator rows must be genuinely blank (no stray
            // black cell): no spans at all.
            for l in &wrapped {
                let text: String = l.spans.iter().map(|s| s.content.to_string()).collect();
                if text.trim().is_empty() {
                    assert!(l.spans.is_empty(), "({width}) 空行不该带任何样式格子: {text:?}");
                }
            }
        }
    }

    #[test]
    fn matched_call_and_result_stack_into_one_card() {
        let p = Palette::default();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"ls"}"#.into(),
                intent: "看目录".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "a.rs".into(),
            },
        ];
        let lines = render(&entries, &p, false, false);
        // Exactly one top edge, one shared middle seam, one bottom edge:
        // two separate cards would produce two of each.
        let edges = lines
            .iter()
            .filter(|l| text_of(std::slice::from_ref(*l)).trim_end().starts_with("+-"))
            .count();
        assert_eq!(edges, 3, "合并后共 3 条横边（上/中/下）: {:?}", text_of(&lines));
        // No type labels anywhere
        let all = text_of(&lines);
        assert!(!all.contains("(in)") && !all.contains("(out)"), "{all}");
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
        // 20 cells wide, container 10 -> 2 wrapped rows, no padding rows of its own
        assert_eq!(estimated_height(&lines, 10), 2);
    }

    #[test]
    fn every_node_is_separated_by_one_blank_row() {
        let p = Palette::default();
        let entries = vec![
            Entry::User { content: "问".into() },
            Entry::Assistant { content: "答".into(), usage: None, reasoning: Some("想".into()) },
            Entry::System { text: "切模型".into(), align: Align::Center },
        ];
        let lines = render(&entries, &p, true, false);
        // Two gaps for three nodes, and never a doubled blank row.
        let blanks = lines
            .iter()
            .filter(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
            .count();
        assert_eq!(blanks, 2, "三个节点之间恰好两条空行: {:?}", text_of(&lines));
        let text = text_of(&lines);
        assert!(!text.contains("\n\n\n"), "不该出现连续两条空行: {text:?}");
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
        // Just the content: the inter-node blank row belongs to the caller.
        assert!(text.contains("答案"));
        assert_eq!(lines.len(), 1, "不该有多余的空思考行: {text:?}");
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
        let tool = LiveActivity::Tool { intent: "正在编译".into() };
        let thinking_live = LiveActivity::Thinking;
        // A tool is running: the model's own explanation takes the live row.
        let during_tool = render_with_live(&[], &p, false, false, 40, &tool, None);
        assert_eq!(during_tool.len(), 1);
        assert_eq!(during_tool[0].spans[0].content, "正在编译");
        // Reasoning streaming: the generic label.
        let thinking = render_with_live(&[], &p, false, false, 40, &thinking_live, None);
        assert_eq!(thinking[0].spans[0].content, "thinking");
        // Idle (content streaming): no live row of its own.
        let after = render_with_live(&[], &p, false, false, 40, &LiveActivity::Idle, Some("你好"));
        assert_eq!(after.len(), 1, "只有正文: {after:?}");
        assert_eq!(after[0].spans[0].content, "你好");
        // The live row is the **bottom-most** history row.
        let flowing = render_with_live(
            &[Entry::Assistant { content: "答".into(), usage: None, reasoning: None }],
            &p, false, false, 40, &tool, None,
        );
        assert_eq!(flowing.last().unwrap().spans[0].content, "正在编译", "live 行必须在最底部");
    }

    #[test]
    fn read_has_no_result_card() {
        let p = Palette::default();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "read".into(),
                args: r#"{"path":"main.rs"}"#.into(),
                intent: "读文件".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "read".into(),
                ok: true,
                result: "1\tfn main() {}".into(),
            },
        ];
        let lines = render(&entries, &p, false, false);
        // Upper card only: one top edge, one bottom edge, no seam.
        let edges = lines
            .iter()
            .filter(|l| text_of(std::slice::from_ref(*l)).trim_end().starts_with("+-"))
            .count();
        assert_eq!(edges, 2, "read 只该有一张卡片: {:?}", text_of(&lines));
        assert!(!text_of(&lines).contains("fn main"), "读到的内容不该重复出现");
    }

    #[test]
    fn empty_tool_result_shows_a_placeholder() {
        let p = Palette::default();
        let e = Entry::ToolResult {
            call_id: "c".into(),
            name: "bash".into(),
            ok: true,
            result: "".into(),
        };
        let all = text_of(&render(&[e], &p, false, false));
        assert!(all.contains("(no output)"), "空结果必须有占位符: {all}");
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
