//! Card primitives: the only set of box drawing in this application.

pub(in crate::ui::tui::zone::main::history) mod chat;
pub(in crate::ui::tui::zone::main::history) mod todo;
pub(in crate::ui::tui::zone::main::history) mod tools;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::ui::tui::text::card::{card_edge, card_edge_labeled, card_row, pad_to};
use crate::ui::tui::theme::{HistoryTheme, Token};
use unicode_width::UnicodeWidthChar;

/// One tool exchange as a card: header, labeled sections, closing edge.
pub(super) struct Card<'a> {
    pub name: &'a str,
    pub args: &'a str,
    pub result: Option<tools::ToolOutcome<'a>>,
    /// The exchange's own copy.
    pub expanded: bool,
    /// The tool is still running: the tail window is what shows.
    pub tail: bool,
    /// The rows a tail window may take.
    pub limit: usize,
}

pub(super) fn tool_card(card: Card<'_>, t: &HistoryTheme, width: usize) -> Vec<Line<'static>> {
    let block = tools::ToolBlock {
        name: card.name,
        args: card.args,
        result: card.result,
        expanded: card.expanded,
        tail: card.tail,
        limit: card.limit,
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

/// A user message: a bar on the left, the body on a filled band.
pub(in crate::ui::tui::zone::main::history) fn user_card(
    content: &str,
    t: &HistoryTheme,
    width: usize,
) -> Vec<Line<'static>> {
    let bg = Style::new().bg(t.get(Token::UserCardBg));
    let bar = Style::new()
        .bg(t.get(Token::UserCardBg))
        .fg(t.get(Token::UserCardBar));
    let mut out = Vec::new();
    out.push(pad_to(Line::from(Span::styled(GUTTER, bar)), width, bg));
    let avail = width.saturating_sub(GUTTER_COLS);
    for line in super::text::markdown::render_markdown(content, t, avail) {
        for row in fold_row(line.spans, avail) {
            let mut spans = vec![Span::styled(GUTTER, bar)];
            let body_fg = match t.get(Token::UserCardText) {
                Color::Reset => Color::White,
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
    out.push(pad_to(Line::from(Span::styled(GUTTER, bar)), width, bg));
    out
}
