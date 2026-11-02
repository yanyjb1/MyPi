//! Completion popup — the candidate list shown above the input container.
//!
//! Why above the container instead of overlaying the input:
//! an overlay would hide the text being typed — exactly what the user reads to pick a candidate.
//!
//! Alignment contract (the whole point of this module's layout):
//! the **first character of every candidate name sits at exactly the cursor's
//! column**. Command rows park their `>` marker in the two columns *left* of that
//! column, so the marker never pushes the name rightward.

use ratatui::text::Line;

use super::engine::{CandidateKind, CompletionPopup, VisibleRow};
use crate::ui::tui::theme::Palette;

// The two columns a command row spends on its selection marker, immediately
// left of the anchor column.
const MARKER_W: usize = 2;

// The popup's rendering result.
pub struct PopupView {
    // Rows, top to bottom (item 0 renders at the very top).
    //
    // Already carry their own leading indent: each row aligns itself to
    // `anchor_col`, so the caller must NOT add any indent of its own.
    //
    // Screen order, never reversed: ↑ moves up,
    // matching what the user sees — no "pressed up, jumped to the top" confusion.
    pub lines: Vec<Line<'static>>,
    // 弹窗锚定的终端绝对列（= 光标列）。命令行的名字就落在这一列上。
    pub anchor_col: usize,
}

// Render the popup. Empty content returns nothing.
//
// `avail` is the popup's row capacity (given by the caller from the space above the container).
pub fn render(
    popup: &CompletionPopup,
    term_w: u16,
    avail: usize,
    anchor_col: usize,
    p: &Palette,
) -> PopupView {
    let visible = popup.visible(avail);
    if visible.is_empty() {
        return PopupView {
            lines: Vec::new(),
            anchor_col: 0,
        };
    }

    // Locked height: pad with blank rows when candidates shrink; the reserved area stays put (no jitter)
    let target = popup.locked_height().unwrap_or(visible.len());

    let inner = crate::ui::tui::zone::main::geometry::inner_width(term_w);

    // Content width feeds the overflow rule: a row that would run past the inner
    // edge pulls the whole popup left so its right edge lands on that edge.
    let popup_w = visible
        .iter()
        .map(|r| row_content_width(r.name, r.detail))
        .max()
        .unwrap_or(0);
    let anchor_col = if anchor_col + popup_w > inner {
        inner.saturating_sub(popup_w)
    } else {
        anchor_col
    };

    // Descriptions are one column across every row: pad names to the widest one so
    // the first character of every description lands on the same x.
    let name_col = visible
        .iter()
        .filter(|r| r.kind == CandidateKind::Command && !r.detail.is_empty())
        .map(|r| crate::ui::tui::text::display_width(r.name))
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible.len());

    for row in visible {
        let VisibleRow {
            kind,
            name,
            hits,
            detail,
            selected,
            is_dir,
        } = row;
        // Every row hangs off the same anchor; `>` and its trailing space are paid
        // for out of the two columns to the left of it.
        let mut spans: Vec<ratatui::text::Span<'static>> = Vec::new();
        spans.push(p.plain(" ".repeat(anchor_col.saturating_sub(MARKER_W))));
        spans.push(p.plain(if selected { "> " } else { "  " }));

        // The name is drawn as a run of spans: matched characters accent, the rest
        // in normal foreground, so the list shows why each candidate is in it.
        spans.extend(name_spans(kind, name, hits, is_dir, p));

        if !detail.is_empty() {
            let pad = name_col.saturating_sub(crate::ui::tui::text::display_width(name));
            spans.push(p.plain(format!("{}  ", " ".repeat(pad))));
            spans.push(p.muted(detail.to_string()));
        }

        lines.push(Line::from(spans));
    }

    // Pad blank placeholder rows up to the locked height
    while lines.len() < target {
        lines.push(Line::from(""));
    }

    PopupView { lines, anchor_col }
}

// Split a candidate name into accented and plain runs.
//
// Two rules, one per kind:
// - `Command` (`/model`): the **whole** name is accent, always. The accent's job
//   there is to separate the command from its description, and there are only a
//   handful of commands — highlighting the typed prefix would just look ragged.
// - `Plain` (paths, profile names, model ids): only the matched characters are
//   accent. Here the list is long and unsorted-by-relevance, so showing which
//   characters matched is what tells the user why a row is in it.
//
// `hits` are char indices into `name`. A directory's trailing `/` is never
// accented under the `Plain` rule: the matcher never looked at it, so accenting
// it would claim a match that did not happen.
fn name_spans(
    kind: CandidateKind,
    name: &str,
    hits: &[usize],
    is_dir: bool,
    p: &Palette,
) -> Vec<ratatui::text::Span<'static>> {
    if kind == CandidateKind::Command {
        return vec![p.accent_span(name.to_string())];
    }
    let chars: Vec<char> = name.chars().collect();
    // The trailing directory slash is decoration, not part of the name proper:
    // the matcher never looked at it, so accenting it would claim a match that
    // never happened.
    let name_len = if is_dir {
        chars.len().saturating_sub(1)
    } else {
        chars.len()
    };
    if hits.is_empty() {
        return vec![p.plain(name.to_string())];
    }
    let hit = |i: usize| hits.binary_search(&i).is_ok();
    let mut spans: Vec<ratatui::text::Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_hit = hit(0);
    for (i, &c) in chars.iter().enumerate() {
        let this_hit = i < name_len && hit(i);
        if this_hit != run_hit {
            if !run.is_empty() {
                spans.push(if run_hit {
                    p.accent_span(std::mem::take(&mut run))
                } else {
                    p.plain(std::mem::take(&mut run))
                });
            }
            run_hit = this_hit;
        }
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(if run_hit {
            p.accent_span(run)
        } else {
            p.plain(run)
        });
    }
    spans
}

// Width of a row's content in display columns, ignoring the leading indent
// (every row is indented to the same anchor, so the indent cancels out when
// comparing rows). Used only for the overflow decision.
fn row_content_width(name: &str, detail: &str) -> usize {
    let name_w = crate::ui::tui::text::display_width(name);
    if detail.is_empty() {
        return name_w + MARKER_W;
    }
    name_w + 2 + crate::ui::tui::text::display_width(detail) + MARKER_W
}
