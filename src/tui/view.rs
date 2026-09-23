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
    // Reasoning currently streaming. Shows "thinking"; disappears once content starts.
    pub live_reasoning: Option<&'a str>,
    // Whether content has started (when the thinking row withdraws).
    pub reasoning_done: bool,
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

// Draw one frame and return where the cursor belongs (container-relative (row, col)) for the caller to place the hardware cursor.
//
// `l` is computed by the caller (`app.rs` needs the same sizes to place the hardware cursor),
// not recomputed here — computing it twice was redundant risk.
pub fn draw(f: &mut Frame, s: &ViewState, l: &tlayout::Layout) -> (u16, u16) {
    let area = f.area();
    let p = &s.palette;
    let inner_w = tlayout::inner_width(area.width);

    // ---- layout: history on top, input container at the bottom, reserved area below (dynamic height) ----
    // The candidate popup no longer overlays: it claims rows from the reserved area,
    // and the history and input container shift up — nothing overlaps anything.
    let [chat_area, container_area, reserved_area] = Layout::vertical([
        Constraint::Min(0),
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
    let chat_lines = chat::render_with_live(
        s.history,
        p,
        s.show_reasoning,
        s.tools_expanded,
        s.live_reasoning,
        s.reasoning_done,
        s.streaming,
    );
    let total = chat::estimated_height(&chat_lines, inner_w);
    let follow = total.saturating_sub(chat_area.height as usize);
    let scroll = if s.scroll_pinned {
        follow
    } else {
        s.chat_scroll.min(follow)
    };
    f.render_widget(
        Paragraph::new(chat_lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        chat_area,
    );

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
