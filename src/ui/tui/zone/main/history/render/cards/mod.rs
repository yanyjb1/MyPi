//! Card primitives: the only set of box drawing in this application.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::super::theme::{HistoryTheme, Token};
use super::tools;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn pad_to(mut line: Line<'static>, width: usize, fill: Style) -> Line<'static> {
    let used: usize = line.spans.iter().map(|s| s.content.as_ref().width()).sum();
    if used < width {
        line.spans
            .push(Span::styled(" ".repeat(width - used), fill));
    }
    line
}

pub(super) fn on_bg(spans: Vec<Span<'static>>, bg: Style) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, bg.patch(sp.style)))
        .collect()
}

pub(crate) fn card_edge(edge: Style, fill: Style, width: usize) -> Line<'static> {
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

pub(crate) fn card_edge_labeled(
    edge: Style,
    fill: Style,
    width: usize,
    label: Line<'static>,
) -> Line<'static> {
    let budget = width.saturating_sub(7);
    if budget == 0 {
        return card_edge(edge, fill, width);
    }
    let line_style = label.style;
    let spans: Vec<Span<'static>> = label
        .spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, line_style.patch(sp.style)))
        .collect();
    let used: usize = spans.iter().map(|s| s.content.as_ref().width()).sum();
    let shown = if used > budget {
        clip_spans(spans, budget)
    } else {
        spans
    };
    let used: usize = shown.iter().map(|s| s.content.as_ref().width()).sum();
    let mut out = vec![Span::styled("+- ", edge)];
    out.extend(shown);
    if used > 0 {
        out.push(Span::styled(" ", edge));
    }
    out.push(Span::styled("-".repeat(budget - used), edge));
    out.push(Span::styled(" -+", edge));
    Line::from(on_bg(out, fill))
}

pub(crate) fn card_row(
    content: Line<'static>,
    edge: Style,
    body_bg: Style,
    width: usize,
) -> Line<'static> {
    let inner = width.saturating_sub(4);
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
    let line_style = content.style;
    let content: Vec<Span<'static>> = content
        .spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, line_style.patch(sp.style)))
        .collect();
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
    Line::from(on_bg(spans, body_bg))
}

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
                let (cut, _) = crate::ui::tui::text::take_width(&sp.content, keep);
                out.push(Span::styled(cut, sp.style));
            }
            break;
        }
    }
    out
}

pub(super) fn tool_card(
    name: &str,
    args: &str,
    result: Option<tools::ToolOutcome<'_>>,
    t: &HistoryTheme,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let block = tools::ToolBlock {
        name,
        args,
        result,
        expanded,
    };
    let edge = block.edge_style(t);
    let bg = block.bg_style(t);
    let mut out = vec![card_edge_labeled(edge, bg, width, block.header(t))];
    for (i, section) in block.sections(t).into_iter().enumerate() {
        if i > 0 || section.label.is_some() {
            out.push(match section.label {
                Some(label) => {
                    card_edge_labeled(edge, bg, width, Line::from(t.fg(label, Token::ToolTitle)))
                }
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

pub(crate) fn exchange_has_two_states(
    name: &str,
    ok: bool,
    result: &str,
    details: Option<&serde_json::Value>,
) -> bool {
    tools::ToolBlock {
        name,
        args: "",
        result: Some(tools::ToolOutcome {
            text: result,
            ok,
            details,
            duration_ms: 0,
        }),
        expanded: false,
    }
    .has_two_states()
}

const GUTTER: &str = "▌ ";
const GUTTER_COLS: usize = 2;

fn fold_row(spans: Vec<Span<'static>>, avail: usize) -> Vec<Vec<Span<'static>>> {
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for sp in spans {
        let style = sp.style;
        let mut buf = String::new();
        for ch in sp.content.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + w > avail && (used > 0 || !cur.is_empty()) {
                if !buf.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut buf), style));
                }
                rows.push(std::mem::take(&mut cur));
                used = 0;
            }
            buf.push(ch);
            used += w;
        }
        if !buf.is_empty() {
            cur.push(Span::styled(buf, style));
        }
    }
    rows.push(cur);
    rows
}

pub(super) fn user_card(content: &str, t: &HistoryTheme, width: usize) -> Vec<Line<'static>> {
    let bg = Style::new().bg(t.get(Token::UserCardBg));
    let mut out = Vec::new();
    out.push(pad_to(
        Line::from(Span::styled(
            "▌ ",
            Style::new()
                .bg(t.get(Token::UserCardBg))
                .fg(t.get(Token::UserCardBar)),
        )),
        width,
        bg,
    ));
    let avail = width.saturating_sub(GUTTER_COLS);
    for line in super::super::text::markdown::render_markdown(content, t, true, avail).0 {
        for row in fold_row(line.spans, avail) {
            let mut spans = vec![Span::styled(
                GUTTER,
                Style::new()
                    .bg(t.get(Token::UserCardBg))
                    .fg(t.get(Token::UserCardBar)),
            )];
            let body_fg = match t.get(Token::UserCardText) {
                ratatui::style::Color::Reset => Color::White,
                c => c,
            };
            for sp in row {
                spans.push(Span::styled(
                    sp.content,
                    Style::new().bg(t.get(Token::UserCardBg)).fg(body_fg),
                ));
            }
            out.push(pad_to(Line::from(spans), width, bg));
        }
    }
    out.push(pad_to(
        Line::from(Span::styled(
            "▌ ",
            Style::new()
                .bg(t.get(Token::UserCardBg))
                .fg(t.get(Token::UserCardBar)),
        )),
        width,
        bg,
    ));
    out
}
