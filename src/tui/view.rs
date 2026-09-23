//! Render orchestration — draws every component onto the terminal and positions the hardware cursor.
//!
//! Division: `app.rs` owns state and events; this only decides how state is drawn.
//! Layout sizes come from `layout.rs`; drawing lives in `components/*`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::entry as entry;
use crate::tui::components::{chat, input, popup, statusline};
use crate::tui::layout as tlayout;
use crate::tui::theme::Palette;

// Everything one frame needs (borrowed, never owned).
pub struct ViewState<'a> {
    // The rendered history entries.
    pub history: &'a [entry::Entry],
    // History scroll offset (rows counted up from the bottom; 0 = follow).
    pub chat_scroll: usize,
    // Whether following the bottom (the user is not browsing elsewhere).
    pub scroll_pinned: bool,
    // Show reasoning expanded (Ctrl+T toggles).
    pub show_reasoning: bool,
    // Global tool-output expansion (Ctrl+O toggles).
    pub tools_expanded: bool,
    // What the user is waiting on: thinking, a tool's intent, or nothing.
    // Drawn as the bottom-most row of the history area.
    pub live: &'a crate::server::events::LiveActivity,
    // Content currently streaming (in-progress slot), rendered with the history area.
    pub streaming: Option<&'a str>,
    // The input's wrap result.
    pub wrapped: &'a crate::tui::text::Wrapped,
    // The cursor's flat char index into the input text.
    pub cursor_char: usize,
    // The spinner's current frame. `None` means idle.
    // (A `streaming` field once existed; rendering only needs the spinner — removed.)
    pub spinner: Option<char>,
    pub model_name: &'a str,
    pub session_name: &'a str,
    pub cwd: &'a str,
    pub git: Option<&'a crate::git::GitStatus>,
    pub ctx_tokens: u64,
    pub ctx_limit: u64,
    pub cost: f64,
    pub currency_symbol: &'a str,
    pub show_cost: bool,
    pub palette: Palette,
    // The completion popup (empty when closed).
    pub popup: &'a crate::tui::path::CompletionPopup,
    // The /resume picker: Some((candidates, highlighted index)). While Some, the reserved area draws it.
    pub resume_pick: Option<(&'a [(i64, String)], usize)>,
}

// Hard-wrap pre-chunked rows to `width` cells.
//
// Every row already exists (a card edge, a card body, a blank separator);
// this only splits the ones too wide, keeping the whole set on one coordinate
// system so scrolling math and drawing cannot disagree.
fn hard_wrap(lines: &[Line<'static>], width: usize) -> Vec<Line<'static>> {
    let w = width.max(1);
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let total: usize = line.spans.iter().map(|s| crate::tui::text::display_width(&s.content)).sum();
        if total <= w {
            out.push(line.clone());
            continue;
        }
        let mut cur: Vec<ratatui::text::Span<'static>> = Vec::new();
        let mut cur_w = 0usize;
        for sp in &line.spans {
            let text = sp.content.to_string();
            let mut buf = String::new();
            for ch in text.chars() {
                let cw = crate::tui::text::display_width(&ch.to_string());
                if cur_w + cw > w {
                    if !buf.is_empty() {
                        cur.push(ratatui::text::Span::styled(std::mem::take(&mut buf), sp.style));
                    }
                    out.push(Line::from(std::mem::take(&mut cur)));
                    cur_w = 0;
                }
                buf.push(ch);
                cur_w += cw;
            }
            if !buf.is_empty() {
                cur.push(ratatui::text::Span::styled(buf, sp.style));
            }
        }
        if !cur.is_empty() {
            out.push(Line::from(cur));
        }
    }
    out
}

// Test-only handle on the wrap pass (the phantom-row regression checks that
// no transcript row overflows the width).
#[cfg(test)]
pub fn hard_wrap_for_test(lines: &[Line<'static>], width: usize) -> Vec<Line<'static>> {
    hard_wrap(lines, width)
}

// Draw one frame and return where the cursor belongs (container-relative (row, col)) for the caller to place the hardware cursor.
//
// `l` is computed by the caller (`app.rs` needs the same sizes to place the hardware cursor),
// not recomputed here — computing it twice was redundant risk.
pub fn draw(f: &mut Frame, s: &ViewState, l: &tlayout::Layout) -> (u16, u16) {
    let area = f.area();
    let p = &s.palette;

    // ---- layout: history on top, input container at the bottom, reserved area below (dynamic height) ----
    // The candidate popup no longer overlays: it claims rows from the reserved area,
    // and the history and input container shift up — nothing overlaps anything.
    // History, then the separator gap, then the container (whose first row
    // *is* the status bar), then the reserved strip.
    let [chat_area, _gap_area, container_area, reserved_area] = Layout::vertical([
        Constraint::Length(l.chat_height),
        Constraint::Length(l.gap_height),
        Constraint::Length(l.container_height),
        Constraint::Length(l.reserved_height),
    ])
    .areas(area);

    // ---- statusline (the container's top edge) ----
    let info = statusline::StatusInfo {
        model_name: s.model_name,
        cwd: s.cwd,
        ctx_tokens: s.ctx_tokens,
        ctx_limit: s.ctx_limit,
        total_cost: s.cost,
        currency_symbol: s.currency_symbol,
        session_name: s.session_name,
        show_cost: s.show_cost,
        git: s.git,
    };
    let status_line = statusline::render(&info, p, area.width, s.spinner);

    // ---- input container: the cursor position comes from here too ----
    let (cur_row_in_body, cur_col_in_body) = s.wrapped.locate(s.cursor_char);
    let iv = input::render(
        &input::InputSpec {
            status_line,
            wrapped: s.wrapped,
            starts: l.first_visible,
            visible_rows: l.visible_rows(),
            cursor_row: cur_row_in_body,
            cursor_col: cur_col_in_body,
            term_w: area.width,
        },
        p,
    );
    let viewport_h = container_area.height as usize;
    let lines: Vec<_> = iv
        .lines
        .into_iter()
        .take(viewport_h)
        .collect();
    f.render_widget(Paragraph::new(lines), container_area);

    // ---- history area ----
    // Follow mode: scroll = total height - visible height; more content scrolls along automatically.
    // After an upward wheel scroll (pinned=false): the viewport pins at chat_scroll,
    // new messages never drag the view; scrolling back to the bottom (offset zeroed) resumes following.
    //
    // Streaming content renders appended to the history: reasoning shows "thinking" (withdrawn
    // once content starts, content takes its place); content appends per delta. All in memory,
    // never touching the DB.
    // Cards span the **full terminal width**: the history area has no
    // borders of its own, so sizing them to `inner_width` (which subtracts
    // the input box's 4 border columns) left a strip of bare terminal
    // background down the right edge of every card.
    let chat_w = area.width as usize;
    let chat_lines = chat::render_with_live(
        s.history,
        p,
        s.show_reasoning,
        s.tools_expanded,
        chat_w,
        s.live,
        s.streaming,
    );
    // Bottom-anchored window, hard-wrapped by us.
    //
    // The history's lower edge is **pinned to the row above the status bar**:
    // the newest content always sits there, and older content scrolls up out
    // of view. Two deliberate departures from `Paragraph::scroll(vec)`:
    //
    //  * the window is computed here, so it can never run off the top (the
    //    old path let a scroll offset push the tail past the viewport and go
    //    blank);
    //  * wrapping is ours and pre-applied, so `estimated_height` and the
    //    drawn geometry agree — with `Paragraph`'s internal wrapping the two
    //    disagreed whenever a row wrapped, which is what made the wheel land
    //    somewhere unrelated to the movement.
    let rows = hard_wrap(&chat_lines, chat_w);
    let viewport = chat_area.height as usize;
    let total = rows.len();
    let max_offset = total.saturating_sub(viewport);
    // `scroll` counts rows up from the bottom (0 = newest visible).
    let offset = if s.scroll_pinned {
        0
    } else {
        s.chat_scroll.min(max_offset)
    };
    let first = max_offset - offset;
    let visible: Vec<Line<'static>> = rows.into_iter().skip(first).take(viewport).collect();
    f.render_widget(Paragraph::new(visible), chat_area);

    // ---- bottom reserved area: the popup float zone (height already in layout; history/input gave way) ----
    // The resume picker wins (its stretched mode); then completion candidates; neither -> one blank row.
    if reserved_area.height > 0 {
        if let Some((items, selected)) = s.resume_pick {
            let lines = crate::tui::components::reserved::render_resume_picker(
                items, selected, area.width, reserved_area.height as usize, p,
            );
            f.render_widget(Paragraph::new(lines), reserved_area);
        } else {
            let pv = popup::render(s.popup, area.width, reserved_area.height as usize, p);
            let lines = if pv.lines.is_empty() {
                vec![Line::from("")]
            } else {
                pv.lines
            };
            f.render_widget(Paragraph::new(lines), reserved_area);
        }
    }

    // ---- cursor (hardware cursor, container-relative coordinates) ----
    let row = (iv.cursor_row).min(viewport_h.saturating_sub(1)) as u16;
    let col = (iv.cursor_col).min(container_area.width.saturating_sub(1) as usize) as u16;
    (row, col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    fn row(text: &str) -> Line<'static> {
        Line::from(Span::raw(text.to_string()))
    }

    #[test]
    fn hard_wrap_splits_only_overwide_rows() {
        let lines = vec![row("abc"), row("0123456789")];
        let wrapped = hard_wrap(&lines, 4);
        // The short row is untouched; the long one becomes three rows.
        let texts: Vec<String> = wrapped
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert_eq!(texts, vec!["abc", "0123", "4567", "89"]);
    }

    #[test]
    fn hard_wrap_never_changes_the_total_character_count() {
        let lines = vec![row("一二三四五六七八九十"), row("ab")];
        let wrapped = hard_wrap(&lines, 4);
        let joined: String = wrapped
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
            .collect();
        assert_eq!(joined, "一二三四五六七八九十ab", "换行不得丢字");
        // Every produced row fits the width (CJK counts as 2 cells).
        for l in &wrapped {
            let w: usize = l.spans.iter().map(|s| crate::tui::text::display_width(&s.content)).sum();
            assert!(w <= 4, "行宽超限: {w}");
        }
    }
}
