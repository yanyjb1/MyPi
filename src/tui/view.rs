//! Render orchestration — draws every component onto the terminal and positions the hardware cursor.
//!
//! Division: `app.rs` owns state and events; this only decides how state is drawn.
//! Layout sizes come from `layout.rs`; drawing lives in `components/*`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::entry as entry;
use crate::tui::components::{input, popup, statusline};
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
    // The block cache lives across frames (App owns it); each frame here
    // only renders the blocks the viewport actually shows.
    pub block_cache: &'a mut crate::tui::transcript::cache::BlockCache,
}

// Test-only handle on the wrap pass (the phantom-row regression checks that
// no transcript row overflows the width).
#[cfg(test)]
pub fn hard_wrap_for_test(lines: &[Line<'static>], width: usize) -> Vec<Line<'static>> {
    hard_wrap(lines, width)
}

#[cfg(test)]
fn hard_wrap(lines: &[Line<'static>], width: usize) -> Vec<Line<'static>> {
    let w = width.max(1);
    let mut out = Vec::new();
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

// Map the visible row window to a block range (with one block of slack
// above). `heights` may contain `usize::MAX` sentinels for not-yet-measured
// blocks — they count as their gap+1 until `rows_for` measures them, which
// is fine: the window edge can only overshoot by a block, and the splice
// below clamps.
fn window_blocks(heights: &[usize], n: usize, first_row: usize, viewport: usize) -> (usize, usize) {
    let mut acc = 0usize;
    let (mut b0, mut b1) = (n, n);
    for (i, h) in heights.iter().enumerate() {
        let size = if *h == usize::MAX { 1 } else { *h + 1 }; // + gap
        if b0 == n && acc + size > first_row {
            b0 = i;
        }
        acc += size;
        if b0 != n && acc > first_row + viewport {
            b1 = (i + 1).min(n);
            break;
        }
    }
    if b0 == n {
        b0 = n.saturating_sub(1); // window past the end (empty roster edge)
    }
    if b1 <= b0 {
        b1 = (b0 + 1).min(n);
    }
    (b0.saturating_sub(1), b1) // one block of slack above
}

// Draw one frame and return where the cursor belongs (container-relative (row, col)) for the caller to place the hardware cursor.
//
// `l` is computed by the caller (`app.rs` needs the same sizes to place the hardware cursor),
// not recomputed here — computing it twice was redundant risk.
pub fn draw(f: &mut Frame, s: &mut ViewState, l: &tlayout::Layout) -> (u16, u16) {
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
    // Block path: sync heights, then render **only** the blocks the
    // viewport touches. Total height comes from the roster (prefix sums),
    // so scrolling never needs the far-away rows at all.
    s.block_cache.sync(
        s.history,
        p,
        s.show_reasoning,
        s.tools_expanded,
        chat_w,
    );
    let total = s.block_cache.total_height();
    let viewport = chat_area.height as usize;
    let max_offset = total.saturating_sub(viewport);
    // `scroll` counts rows up from the bottom (0 = newest visible).
    let offset = if s.scroll_pinned { 0 } else { s.chat_scroll.min(max_offset) };
    let first_row = max_offset - offset; // absolute top row of the window

    // Which blocks does [first_row, first_row+viewport) touch? Walk the
    // height roster — O(blocks), no rendering — then materialize that
    // slice (+1 block of slack above for smooth wheeling).
    let n_blocks = crate::tui::transcript::blocks::blocks(s.history).len();
    let (b0, b1) = window_blocks(s.block_cache.heights_slice(), n_blocks, first_row, viewport);
    let (block_rows, rows_above) =
        s.block_cache.rows_for(s.history, p, s.show_reasoning, s.tools_expanded, b0..b1);
    // Splice: rows above the window are dropped; what remains paints.
    let skip = first_row.saturating_sub(rows_above);
    let mut visible: Vec<Line<'static>> = Vec::with_capacity(viewport);
    let mut it = block_rows.into_iter().skip(skip);
    for _ in 0..viewport {
        match it.next() {
            Some(l) => visible.push(l),
            None => break,
        }
    }
    // ---- live tail (bottom-most history rows) ----
    // Thinking / tool intent draw as a muted italic label; in-flight
    // content streams in at full weight (it is the final answer).
    // Appended below the newest block; the follow-bottom window keeps it
    // on screen because it rides the same `total` bookkeeping… except it
    // is not a block, so splice it when the window reaches the bottom.
    if max_offset - offset == 0 || first_row + viewport > rows_above {
        match s.live {
            crate::server::events::LiveActivity::Thinking => visible.push(Line::styled(
                "thinking",
                ratatui::style::Style::new().fg(p.muted).add_modifier(ratatui::style::Modifier::ITALIC),
            )),
            crate::server::events::LiveActivity::Tool { intent } => {
                let label = if intent.trim().is_empty() { "working" } else { intent.as_str() };
                visible.push(Line::styled(
                    label.to_string(),
                    ratatui::style::Style::new().fg(p.muted).add_modifier(ratatui::style::Modifier::ITALIC),
                ));
            }
            crate::server::events::LiveActivity::Idle => {}
        }
        if let Some(t) = s.streaming
            && !t.is_empty()
        {
            visible.extend(crate::tui::transcript::components::chat::render_streaming(t, p));
        }
    }
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
