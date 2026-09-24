//! Card scaffolding + tool cards — the bordered, true-black visual system.
//!
//! One set of primitives (`card_edge` / `card_row` / `on_bg` / `pad_to`)
//! draws every card; the tool views (exchange / request / result / payload)
//! compose them. Restyling the cards happens here and nowhere else.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::entry::ToolView;
use crate::tui::highlight;
use crate::tui::theme::ColorToken;
use crate::tui::theme::Palette;
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------
// card scaffolding
// ---------------------------------------------------------------------------

// Pad `line` out to `width` display cells with `fill`.
//
// Every card body goes through this: a background only covers the cells it
// actually paints, so an unpadded row would end mid-card.
pub(super) fn pad_to(mut line: Line<'static>, width: usize, fill: Style) -> Line<'static> {
    let used: usize = line.spans.iter().map(|s| s.content.as_ref().width()).sum();
    if used < width {
        line.spans
            .push(Span::styled(" ".repeat(width - used), fill));
    }
    line
}

// Give every cell of a card row the card's background.
//
// A background only covers the cells it paints, and painting it on the
// padding alone left the text sitting on the terminal default — so a card
// looked black in the gaps and *not* black behind the words. Every span
// (frame included) goes through here.
pub(super) fn on_bg(spans: Vec<Span<'static>>, bg: Style) -> Vec<Span<'static>> {
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
pub(super) fn card_edge(edge: Style, fill: Style, width: usize) -> Line<'static> {
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
pub(super) fn card_row(
    content: Line<'static>,
    edge: Style,
    body_bg: Style,
    width: usize,
) -> Line<'static> {
    let inner = width.saturating_sub(4);
    // Defensive: never let stray escape bytes reach the terminal from inside a
    // card. The tools strip their own output, but file contents and rows read
    // back from an older DB can still carry them.
    let clean_style = content.style;
    let content = Line::from(
        content
            .spans
            .into_iter()
            .map(|sp| Span::styled(crate::ansi::strip_ansi(&sp.content), sp.style))
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
pub(super) fn clip_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
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

// ---------------------------------------------------------------------------

// tool cards
// ---------------------------------------------------------------------------

/// Result-card edge color: success/error token (theme-owned, not literal
/// green/red — a mid-session theme switch recolors the next frame).
fn result_edge(ok: bool) -> Color {
    crate::tui::theme::theme().color(if ok {
        ColorToken::ToolSuccessBg
    } else {
        ColorToken::ToolErrorBg
    })
}

/// Readable foreground on the card background (omp's contrast rule).
fn contrast_on(bg: Color) -> Color {
    crate::tui::theme::contrast_text_on(bg)
}

/// Diff row background: the diff color dimmed toward the card background,
/// so full-width rows stay quiet on the black card.
fn dim_bg(fg: Color, card: Color) -> Color {
    let (fr, fg_, fb) = match fg {
        Color::Rgb(r, g, b) => (r as u32, g as u32, b as u32),
        _ => return card,
    };
    let (cr, cg, cb) = match card {
        Color::Rgb(r, g, b) => (r as u32, g as u32, b as u32),
        _ => return card,
    };
    // 25% color + 75% card: readable tint, not a shout.
    Color::Rgb(
        ((cr * 3 + fr) / 4) as u8,
        ((cg * 3 + fg_) / 4) as u8,
        ((cb * 3 + fb) / 4) as u8,
    )
}

// Whether a result deserves its own card.
//
// Reading is not a change: `read`'s result is the file it just showed, and
// the request card already names the path, so a result card would repeat it.
// Every other tool did something worth confirming.
pub(super) fn result_card_visible(name: &str) -> bool {
    name != "read"
}

// Default fold threshold for tool output (lines). Per-tool overrides: [`collapse_limit`].
const DEFAULT_COLLAPSE_LINES: usize = 5;

// Per-tool thresholds: edit tools are lenient (a screenful of diff is worth showing directly).
// Unregistered tools fall back to the default 5 lines.
pub(super) fn collapse_limit(tool: &str) -> usize {
    match tool {
        "edit" | "mass_edit" => 14,
        _ => DEFAULT_COLLAPSE_LINES,
    }
}

/// Does this exchange render **differently** under Ctrl+O (expand)?
///
/// The single source of truth for "does the tools_expanded switch matter
/// for this block": a card folds only when a visible result's plain-text
/// body exceeds its per-tool fold threshold, and diffs never fold. The
/// cache consults this before rendering so single-state blocks are
/// rendered (and stored) exactly once.
pub(in crate::tui::transcript) fn exchange_has_two_states(
    name: &str,
    ok: bool,
    result: &str,
) -> bool {
    if !result_card_visible(name) {
        return false;
    }
    match ToolView::synthesize(name, ok, result) {
        ToolView::Diff { .. } => false,
        ToolView::Plain { text } => text.lines().count() > collapse_limit(name),
    }
}

// Two cards glued into one: the call's content, the seam, the result's content.
//
// A tool call and its result are one exchange, not two messages — drawing
// them as two separate boxes (each with its own top *and* bottom edge) made
// a single `ls` cost six rows. Here the middle edge is shared, and the
// result's edges take the success/failure color so the outcome still reads
// at a glance. No type labels: the content says what it is.
pub(super) fn tool_exchange(
    name: &str,
    args: &str,
    ok: bool,
    result: &str,
    p: &Palette,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let edge = Style::new().fg(p.accent);
    let out_edge = Style::new().fg(result_edge(ok));
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
pub(super) fn tool_request_card(
    name: &str,
    args: &str,
    p: &Palette,
    width: usize,
) -> Vec<Line<'static>> {
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
pub(super) fn tool_result_card(
    name: &str,
    ok: bool,
    result: &str,
    p: &Palette,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    // An unpaired result (interrupted call): colour tells the outcome, the
    // content tells the rest. No labels, same as every other card.
    let edge = Style::new().fg(result_edge(ok));
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
pub(super) fn result_lines(
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
        ToolView::Diff {
            deletions,
            insertions,
        } => {
            // Deletions above, insertions below: red and green backgrounds
            // (vscode style). Kept as row styles; `card_row` folds them into
            // the spans so they survive the frame.
            let t = crate::tui::theme::theme();
            for d in deletions {
                out.push(Line::styled(
                    format!("- {d}"),
                    Style::new()
                        .fg(contrast_on(p.black))
                        .bg(dim_bg(t.color(ColorToken::ToolDiffRemoved), p.black)),
                ));
            }
            for i in insertions {
                out.push(Line::styled(
                    format!("+ {i}"),
                    Style::new()
                        .fg(contrast_on(p.black))
                        .bg(dim_bg(t.color(ColorToken::ToolDiffAdded), p.black)),
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
pub(super) fn payload_lines(name: &str, args: &str, p: &Palette) -> Vec<Line<'static>> {
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
            let t = crate::tui::theme::theme();
            let mut out: Vec<Line<'static>> = vec![Line::from(
                t.fg(ColorToken::ToolTitle, format!("path: {path}")),
            )];
            if let Some(old) = v.get("old").and_then(|c| c.as_str()) {
                out.push(Line::from(t.fg(ColorToken::ToolDiffRemoved, "- old:")));
                for l in old.lines() {
                    out.extend(highlight::highlight(l, lang));
                }
            }
            if let Some(new) = v.get("new").and_then(|c| c.as_str()) {
                out.push(Line::from(t.fg(ColorToken::ToolDiffAdded, "+ new:")));
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

// User message: a card on the true-black background, white text.
//
// The black is the palette's truecolor black (`0,0,0`), never the ANSI
// indexed black — indexed black is what terminals map to gray, which is
// exactly the look we are avoiding here.
pub(super) fn user_card(content: &str, p: &Palette, width: usize) -> Vec<Line<'static>> {
    let bg = Style::new().bg(p.black);
    let mut out = Vec::new();
    // Breathing room: a blank row above and below, still on the accent gutter
    // and the true-black background, so the card reads as one solid block.
    out.push(pad_to(
        Line::from(Span::styled("▌ ", Style::new().bg(p.black).fg(p.accent))),
        width,
        bg,
    ));
    for line in crate::tui::components::markdown::render_markdown(content, p) {
        // Force the card's own foreground/background: markdown may have
        // decided on a color for a code span, but a user message is
        // uniformly black-on-… white-on-black.
        let mut spans = vec![Span::styled("▌ ", Style::new().bg(p.black).fg(p.accent))];
        for sp in line.spans {
            let t = crate::tui::theme::theme();
            let body_fg = match t.color(ColorToken::UserMessageText) {
                ratatui::style::Color::Reset => Color::White,
                c => c,
            };
            spans.push(Span::styled(
                sp.content,
                Style::new().bg(p.black).fg(body_fg),
            ));
        }
        out.push(pad_to(Line::from(spans), width, bg));
    }
    out.push(pad_to(
        Line::from(Span::styled("▌ ", Style::new().bg(p.black).fg(p.accent))),
        width,
        bg,
    ));
    out
}

/// Test hook: markdown-in-user-card integration lives in markdown.rs tests.
#[cfg(test)]
pub(crate) fn user_card_public_for_test(
    content: &str,
    p: &Palette,
    width: usize,
) -> Vec<Line<'static>> {
    user_card(content, p, width)
}

// (the trailing blank row between nodes is `section_gap`, added by the caller)
