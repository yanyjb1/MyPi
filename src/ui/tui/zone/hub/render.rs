// The hub page: the child list on top, the chosen child's newest blocks under it.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::json;

use super::zone::{HubZone, Row, title};
use crate::ui::tui::lang::Lang;
use crate::ui::tui::text::{bytes as byte_size, short};
use crate::ui::tui::theme::{Accent, fixed};
use crate::ui::tui::zone::page::{Hint, Panel, divider};

// Rows one child takes: its title, its figures, the blank under them.
const CHILD: usize = 3;

// The narrowest preview worth drawing at all.
const FLOOR: usize = 3;

// The seam between the list and the preview: a blank, a rule, a blank.
const SEAM: usize = 3;

// One sentence of this language, its `{name}` holes filled.
fn text(lang: Lang, key: &str, args: serde_json::Value) -> String {
    crate::i18n::render(lang.code(), key, &args)
}

// A span in one fixed color.
fn fg(text: impl Into<String>, color: ratatui::style::Color) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(color))
}

pub(super) fn lines(z: &mut HubZone) -> Vec<Line<'static>> {
    let Some(size) = z.size() else {
        return Vec::new();
    };
    let lang = z.lang();
    let accent = z.accent();

    let header = text(
        lang,
        "hub_title",
        json!({"count": z.picker().visible_len(), "running": z.running()}),
    );

    let panel = Panel {
        size,
        title: header,
        // A handful of children need no filter word; the rows it would take go to the preview.
        filter: None,
        note: Some(text(lang, "hub_note", json!({}))),
        extra: Vec::new(),
        empty: empty(z, lang),
        hints: hints(lang, z.peek()),
        accent,
    };

    let room = panel.body_rows();
    let cols = panel.content();
    let count = z.picker().visible_len();
    if count == 0 {
        return panel.draw(Vec::new());
    }

    let (list_rows, preview_rows) = split(room);
    let children = list_rows / CHILD;
    let mut top = z.picker().scroll().min(count.saturating_sub(1));
    if z.picker().selected() < top {
        top = z.picker().selected();
    }
    if z.picker().selected() >= top + children {
        top = (z.picker().selected() + 1).saturating_sub(children);
    }
    z.picker_mut().set_scroll(top);

    let mut body: Vec<Line<'static>> = Vec::with_capacity(room);
    let shown = children.min(count - top);
    for i in top..top + shown {
        let Some(row) = z.picker().row_at(i) else {
            continue;
        };
        let selected = i == z.picker().selected();
        body.extend(row_lines(row, selected, &accent, lang));
    }
    if preview_rows == 0 {
        return panel.draw(body);
    }

    // The list keeps its own band whatever it holds, so the seam never rides up and down.
    while body.len() < list_rows {
        body.push(Line::default());
    }
    body.push(Line::default());
    body.push(divider(cols));
    body.push(Line::default());
    body.extend(preview(z, preview_rows, &accent, lang));
    panel.draw(body)
}

// The body, cut into the list's share and the preview's, with the seam that belongs to neither.
// The list may take half the room; a short list gives whatever is left to the preview.
fn split(room: usize) -> (usize, usize) {
    let least = CHILD + SEAM + FLOOR;
    if room < least {
        return (room, 0);
    }
    let list = (room / 2).max(CHILD) / CHILD * CHILD;
    (list, room - list - SEAM)
}

// What the list says when it has nothing to show.
fn empty(z: &HubZone, lang: Lang) -> Option<Line<'static>> {
    if let Some(error) = z.error() {
        let word = text(lang, "hub_read_failed", json!({"detail": error}));
        return Some(Line::from(fg(word, fixed::ERR)));
    }
    if z.picker().visible_len() > 0 {
        return None;
    }
    let key = if z.loaded() {
        "hub_empty"
    } else {
        "hub_empty_loading"
    };
    Some(Line::from(fg(text(lang, key, json!({})), fixed::MUTED)))
}

// The three rows of one child: its title, its figures, then air.
fn row_lines(row: &Row, selected: bool, accent: &Accent, lang: Lang) -> Vec<Line<'static>> {
    // The cursor is the only position indicator, and its blank placeholder keeps every title aligned.
    let cursor = if selected { "> " } else { "  " };
    let cursor_color = if selected {
        accent.color()
    } else {
        fixed::MUTED
    };
    // A running child carries the live color; a finished one is grey.
    let title_color = if row.running {
        accent.color()
    } else if selected {
        fixed::BODY
    } else {
        fixed::MUTED
    };
    let mut spans = vec![fg(cursor, cursor_color)];
    spans.push(Span::styled(
        title(row),
        Style::new().fg(title_color).add_modifier(Modifier::BOLD),
    ));

    let mut meta: Vec<Span<'static>> = vec![fg("  ", fixed::MUTED)];
    let dot = || fg(" · ", fixed::MUTED);
    meta.push(fg(
        text(lang, "hub_rounds", json!({"n": row.rounds})),
        fixed::DIM,
    ));
    meta.push(dot());
    meta.push(fg(byte_size(row.bytes.max(0) as u64), fixed::DIM));
    if let Some(cwd) = row.cwd.as_deref() {
        meta.push(dot());
        meta.push(fg(short(cwd), fixed::DIM));
    }
    vec![Line::from(spans), Line::from(meta), Line::default()]
}

// The chosen child's blocks, the one under the preview's cursor marked.
fn preview(z: &HubZone, rows: usize, accent: &Accent, lang: Lang) -> Vec<Line<'static>> {
    if rows == 0 {
        return Vec::new();
    }
    let ids = z.order();
    if ids.is_empty() {
        let word = text(lang, "hub_preview_empty", json!({}));
        return vec![Line::from(fg(word, fixed::DIM))];
    }
    let cursor = z.cursor();
    let at = cursor
        .and_then(|id| ids.iter().position(|each| *each == id))
        .unwrap_or(ids.len() - 1);
    // The window ends at the cursor and reaches back: a reader follows the newest block.
    let start = at.saturating_sub(rows.saturating_sub(1));
    ids[start..=at]
        .iter()
        .map(|id| match z.block(*id) {
            Some(block) => paint(block, Some(*id) == cursor, accent, lang),
            None => Line::from(fg(format!("  #{id}"), fixed::DIM)),
        })
        .collect()
}

// One block: its role, then the first line of what it says.
fn paint(block: &super::zone::Block, on: bool, accent: &Accent, lang: Lang) -> Line<'static> {
    let (role, rest) = block
        .text
        .split_once(": ")
        .unwrap_or(("", block.text.as_str()));
    let role = match role.is_empty() {
        true => String::new(),
        false => text(lang, &format!("role_{role}"), json!({})),
    };
    let color = role_color(&block.kind, accent);
    let mut spans = vec![fg(if on { "> " } else { "  " }, fixed::DIM)];
    if !role.is_empty() {
        spans.push(Span::styled(
            role,
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ));
        spans.push(fg(": ", fixed::DIM));
    }
    spans.push(fg(rest.to_string(), fixed::BODY));
    Line::from(spans)
}

// A block's role to its color; the same five the tree uses.
fn role_color(kind: &str, accent: &Accent) -> ratatui::style::Color {
    match kind {
        "user" => accent.color(),
        "assistant" => fixed::TREE_ASSISTANT,
        "tool_call" => fixed::TREE_TOOL_CALL,
        "tool_result" => fixed::TREE_TOOL_RESULT,
        "reasoning" => fixed::TREE_REASONING,
        _ => fixed::DIM,
    }
}

// Every key the page answers to; the first three lead because a narrow frame keeps the front.
fn hints(lang: Lang, peek: bool) -> Vec<Hint> {
    let hint = |keys: &'static str, key: &str| Hint::new(keys, text(lang, key, json!({})));
    // Inside the preview Enter means "back to the list", so it takes the word Esc carries there.
    let enter = if peek { "hint_back" } else { "hint_peek" };
    vec![
        hint("↑↓", "hint_select"),
        hint("Enter", enter),
        hint("Esc", "hint_back"),
        hint("Ctrl+X", "hint_interrupt"),
        hint("PgUp/PgDn", "hint_page"),
        hint("Home/End", "hint_ends"),
    ]
}
