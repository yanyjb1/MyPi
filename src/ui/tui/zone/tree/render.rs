// The tree page: the page chassis, filled with one row per block.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::json;
use unicode_width::UnicodeWidthStr;

use super::layout::COL_STEP;
use super::rows::Row;
use super::zone::TreeZone;
use crate::ui::tui::lang::Lang;
use crate::ui::tui::theme::{Accent, fixed};
use crate::ui::tui::zone::TermSize;
use crate::ui::tui::zone::page::{Filter, Hint, Panel};

// Vertical bar descending from an ancestor fork.
const BAR: char = '│';
// Connector: more siblings follow.
const TEE: char = '├';
// Connector: last sibling.
const CORNER: char = '└';
// Horizontal segment.
const DASH: char = '─';
// Truncation marker.
const ELLIPSIS: char = '…';

// One sentence of this language, its `{name}` holes filled.
fn text(lang: Lang, key: &str, args: serde_json::Value) -> String {
    crate::i18n::render(lang.code(), key, &args)
}

pub(super) fn lines(z: &mut TreeZone) -> Vec<Line<'static>> {
    let Some(size) = z.size() else {
        return Vec::new();
    };
    let lang = z.lang();
    let accent = z.accent();

    let scope = match z.only_user() {
        true => text(lang, "tree_scope_user", json!({})),
        false => text(lang, "tree_scope_all", json!({})),
    };
    let header = text(
        lang,
        "tree_title",
        json!({"count": z.picker().visible_len(), "turns": z.turns(), "scope": scope}),
    );

    let panel = Panel {
        size,
        title: header,
        filter: Some(Filter {
            text: z.picker().filter().to_string(),
            placeholder: text(lang, "tree_filter_hint", json!({})),
            caption: text(lang, "tree_filter_caption", json!({})),
        }),
        extra: Vec::new(),
        empty: empty(z, lang),
        hints: hints(lang, z.cherish(), z.only_user()),
        accent,
    };

    let rows = panel.body_rows();
    let cols = panel.content();
    let count = z.picker().visible_len();
    let mut top = z.picker().scroll().min(count.saturating_sub(1));
    if z.picker().selected() < top {
        top = z.picker().selected();
    }
    let shown = rows.min(count.saturating_sub(top));
    if z.picker().selected() >= top + shown && shown > 0 {
        top = (z.picker().selected() + 1).saturating_sub(rows);
    }
    z.picker_mut().set_scroll(top);

    let mut body = Vec::with_capacity(shown);
    for i in top..top + shown {
        let Some(row) = z.picker().row_at(i) else {
            continue;
        };
        let line = paint(row, cols, lang, &accent);
        body.push(highlight(line, i == z.picker().selected(), cols));
    }
    panel.draw(body)
}

// What the page says when it has nothing to show.
fn empty(z: &TreeZone, lang: Lang) -> Option<Line<'static>> {
    if z.picker().visible_len() > 0 {
        return None;
    }
    let key = if z.picker().filter().is_empty() {
        "tree_empty"
    } else {
        "tree_empty_filtered"
    };
    Some(Line::from(Span::styled(
        text(lang, key, json!({})),
        Style::new().fg(fixed::MUTED),
    )))
}

// Every key the tree answers to; three of them name the state they would put the page in.
// The first three lead because a narrow terminal keeps the front of this list and drops the tail.
pub(super) fn hints(lang: Lang, cherish: bool, only_user: bool) -> Vec<Hint> {
    let hint = |keys: &'static str, key: &str| Hint::new(keys, text(lang, key, json!({})));
    let (enter, shift) = if cherish {
        ("hint_focus", "hint_rewind")
    } else {
        ("hint_rewind", "hint_focus")
    };
    let tab = if only_user {
        "hint_all_blocks"
    } else {
        "hint_only_user"
    };
    vec![
        hint("↑↓", "hint_select"),
        hint("Enter", enter),
        hint("Esc", "hint_back"),
        hint("PgUp/PgDn", "hint_page"),
        hint("Home/End", "hint_ends"),
        hint("Shift+Enter", shift),
        hint("Tab", tab),
        hint("Ctrl+U", "hint_clear"),
    ]
}

// Rows the body may take on this size, with these hints at the bottom.
pub(super) fn body_rows(size: TermSize, hints: &[Hint]) -> usize {
    crate::ui::tui::zone::page::body_rows(size, true, 0, hints)
}

// Inverts the whole row, keeping its colors.
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

// Prefix: ancestor bars, this level's connector and indent blanks.
// Structure runs and blanks alternate as spans, not one span per character.
fn prefix(geom: &super::layout::RowGeom, spans: &mut Vec<Span<'static>>, axis: Style) {
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

// Prefix plus the block text, truncated to the column width.
// Bars and dashes are dim; the role name takes its role color, the summary the body color.
fn paint(row: &Row, cols: usize, lang: Lang, accent: &Accent) -> Line<'static> {
    let axis = Style::new().fg(fixed::TREE_AXIS);
    let mut spans: Vec<Span<'static>> = Vec::new();
    prefix(&row.geom, &mut spans, axis);

    // Body: the role in the role's own color, then the summary in the body color.
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let body_cols = cols.saturating_sub(used).max(1);
    let (role, rest) = split_role(&row.text);
    let role = match role.is_empty() {
        true => String::new(),
        false => text(lang, &format!("role_{}", role), json!({})),
    };
    // Columns taken by the role name plus ": ".
    let role_cols = role.as_str().width() + 2;

    if !role.is_empty() && role_cols + 1 < body_cols {
        let style = role_style(&row.kind, accent);
        spans.push(Span::styled(role, style));
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

// Splits `"user: text"` into `("user", "text")`; without a colon the role is empty.
fn split_role(text: &str) -> (&str, &str) {
    match text.split_once(": ") {
        Some((r, rest)) => (r, rest),
        None => ("", text),
    }
}

// Everything the filter can match on: the role as the page writes it, and the summary.
pub(super) fn haystack(row: &Row, lang: Lang) -> String {
    let (role, rest) = split_role(&row.text);
    let role = match role.is_empty() {
        true => String::new(),
        false => text(lang, &format!("role_{role}"), json!({})),
    };
    format!("{role} {rest}").to_lowercase()
}

// Role to color; the five roles are pairwise distinct.
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

// Truncates to `max` displayed columns, appending the ellipsis when cut.
fn fit(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let (cut, _) = crate::ui::tui::text::take_width(s, max.saturating_sub(1));
    let mut out = cut;
    out.push(ELLIPSIS);
    out
}
