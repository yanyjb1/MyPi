//! Node tree rendering: geometry to characters.
//!
//! One block per line, never wrapped. Branch structure lives only in the
//! leading prefix; the rules are in [`super::layout`].
//!
//! Widths are displayed columns (`unicode_width`), not `chars().count()`:
//! wide characters occupy two columns.
//!
//! Each of the five roles gets its own color from [`fixed`]; user nodes use accent.

use super::layout::{COL_STEP, RowGeom};
use super::rows::Row;
use crate::ui::tui::theme::{Accent, fixed};
use crate::ui::tui::zone::TermSize;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// Vertical bar descending from an ancestor fork.
const BAR: char = '│';
/// Connector: more siblings follow.
const TEE: char = '├';
/// Connector: last sibling.
const CORNER: char = '└';
/// Horizontal segment.
const DASH: char = '─';
/// Truncation marker.
const ELLIPSIS: char = '…';

/// Draws the visible page, one block per row.
pub fn rows(
    rows: &[Row],
    selected: usize,
    size: Option<TermSize>,
    scroll: &mut usize,
    accent: &Accent,
) -> Vec<Line<'static>> {
    let height = size.map(|s| s.rows as usize).unwrap_or(24);
    let cols = size.map(|s| s.cols as usize).unwrap_or(80);
    // Top row is reserved for the hint line.
    let body = height.saturating_sub(1);

    if rows.is_empty() {
        return vec![Line::from(" （空）")];
    }

    // One row per block: the scroll window counts rows.
    if selected < *scroll {
        *scroll = selected;
    } else if selected >= *scroll + body {
        *scroll = selected + 1 - body;
    }
    *scroll = (*scroll).min(rows.len().saturating_sub(body));

    let mut out = Vec::with_capacity(body + 1);
    out.push(Line::from(" ↑↓ 选块   Enter 回到这块   Esc 关 "));
    for (i, row) in rows.iter().enumerate().skip(*scroll).take(body) {
        out.push(highlight(paint(row, cols, accent), i == selected, cols));
    }
    out
}

/// Inverts the whole row, keeping its colors.
fn highlight(line: Line<'static>, on: bool, cols: usize) -> Line<'static> {
    if !on {
        return line;
    }
    let used = line.width();
    let mut spans = line.spans;
    if used < cols {
        spans.push(Span::raw(" ".repeat(cols - used)));
    }
    Line::from(spans).style(Style::new().add_modifier(Modifier::REVERSED))
}

/// Prefix: ancestor bars + this level's connector + indent blanks.
///
/// Structure runs and blanks alternate as spans, not one span per character.
fn prefix(geom: &RowGeom, spans: &mut Vec<Span<'static>>, axis: Style) {
    let conn_level = geom.connector_level();
    let mut bars = String::new();
    let mut blanks = String::new();
    for i in 0..geom.prefix_cols() {
        let level = i / COL_STEP;
        let pos = i % COL_STEP;
        let ch = if let Some(show) = geom.gutter(level) {
            // Ancestor fork: the bar occupies only the first column of its level.
            (pos == 0 && show).then_some(BAR)
        } else if conn_level == Some(level) {
            match pos {
                0 => Some(if geom.connector == Some(true) {
                    CORNER
                } else {
                    TEE
                }),
                1 => Some(DASH),
                _ => None,
            }
        } else {
            None
        };
        match ch {
            Some(c) => {
                if !blanks.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut blanks)));
                }
                bars.push(c);
            }
            None => {
                if !bars.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut bars), axis));
                }
                blanks.push(' ');
            }
        }
    }
    if !blanks.is_empty() {
        spans.push(Span::raw(std::mem::take(&mut blanks)));
    }
    if !bars.is_empty() {
        spans.push(Span::styled(std::mem::take(&mut bars), axis));
    }
}

/// Prefix plus the block text, truncated to the column width.
///
/// Bars and dashes are dim; the role name takes its role color, the summary the body color.
fn paint(row: &Row, cols: usize, accent: &Accent) -> Line<'static> {
    let axis = Style::new().fg(fixed::TREE_AXIS);
    let mut spans: Vec<Span<'static>> = Vec::new();
    prefix(&row.geom, &mut spans, axis);

    // Body: `role: summary`, role name in the role color, summary in body color.
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let body_cols = cols.saturating_sub(used).max(1);
    let (role, rest) = split_role(&row.text);
    // Columns taken by the role name plus ": ".
    let role_cols = role.width() + 2;
    if role_cols + 1 < body_cols {
        spans.push(Span::styled(
            role.to_string(),
            role_style(&row.kind, accent),
        ));
        spans.push(Span::raw(": "));
        let rest_cols = body_cols - role_cols;
        spans.push(Span::styled(
            fit(rest, rest_cols),
            Style::new().fg(fixed::BODY),
        ));
    } else {
        // Too narrow for the role name: truncate the whole text in the role color.
        spans.push(Span::styled(
            fit(&row.text, body_cols),
            role_style(&row.kind, accent),
        ));
    }
    Line::from(spans)
}

/// Splits `"user: text"` into `("user", "text")`; without a colon the role is empty.
fn split_role(text: &str) -> (&str, &str) {
    match text.split_once(": ") {
        Some((r, rest)) => (r, rest),
        None => ("", text),
    }
}

/// Role to color; the five roles are pairwise distinct.
fn role_style(kind: &str, accent: &Accent) -> Style {
    let color = match kind {
        "user" => accent.color(),
        "assistant" => fixed::TREE_ASSISTANT,
        "tool_call" => fixed::TREE_TOOL_CALL,
        "tool_result" => fixed::TREE_TOOL_RESULT,
        "reasoning" => fixed::TREE_REASONING,
        _ => fixed::DIM,
    };
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

/// Truncates to `max` displayed columns, appending the ellipsis when cut.
fn fit(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let (cut, _) = crate::ui::tui::text::take_width(s, max.saturating_sub(1));
    let mut out = cut;
    out.push(ELLIPSIS);
    out
}
