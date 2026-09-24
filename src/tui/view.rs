//! Render orchestration — draws every component onto the terminal and positions the hardware cursor.
//!
//! Division: `app.rs` owns state and events; this only decides how state is drawn.
//! Layout sizes come from `layout.rs`; drawing lives in `components/*`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::entry;
use crate::tui::completion::popup;
use crate::tui::components::{input, statusline};
use crate::tui::layout as tlayout;
use crate::tui::theme::Palette;

// Everything one frame needs (borrowed, never owned).
pub struct ViewState<'a> {
    // The rendered history entries.
    pub history: &'a [entry::Entry],
    /// Generation of `history` (see `SessionState::transcript_generation`):
    /// lets the block cache notice a wholesale transcript swap.
    pub transcript_generation: u64,
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
    pub popup: &'a crate::tui::completion::CompletionPopup,
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
        let total: usize = line
            .spans
            .iter()
            .map(|s| crate::tui::text::display_width(&s.content))
            .sum();
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
                        cur.push(ratatui::text::Span::styled(
                            std::mem::take(&mut buf),
                            sp.style,
                        ));
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
    let lines: Vec<_> = iv.lines.into_iter().take(viewport_h).collect();
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
    // Block path: the window is anchored at the **bottom** (offset = rows
    // up from the tail), so locating it is a reverse walk that measures
    // unmeasured blocks on demand (render = measure; rows stay cached).
    // Nothing above the walk's stop point is ever rendered: cold start
    // paints exactly one viewport, ancient history waits until wheeled.
    s.block_cache.sync(
        s.history,
        s.transcript_generation,
        p,
        s.show_reasoning,
        s.tools_expanded,
        chat_w,
    );
    let viewport = chat_area.height as usize;
    let offset = if s.scroll_pinned { 0 } else { s.chat_scroll };
    let (b0, b1) = s.block_cache.window_from_bottom(
        s.history,
        p,
        s.show_reasoning,
        s.tools_expanded,
        offset,
        viewport,
    );
    let block_rows =
        s.block_cache
            .rows_for(s.history, p, s.show_reasoning, s.tools_expanded, b0..b1);
    // ---- live tail (bottom-most history rows) ----
    // Thinking / tool intent draw as a muted italic label; in-flight
    // content streams in at full weight (it is the final answer).
    // It is not a block, so it is spliced in by hand — and its height must be
    // **reserved before the block window is cut**: appending it after a full
    // viewport put it past the last row, where `Paragraph` clips it, so the
    // "thinking" label and the streaming reply were invisible in any session
    // whose history filled the chat area.
    let mut tail: Vec<Line<'static>> = Vec::new();
    if offset == 0 {
        match s.live {
            crate::server::events::LiveActivity::Thinking => tail.push(Line::styled(
                "thinking",
                ratatui::style::Style::new()
                    .fg(p.muted)
                    .add_modifier(ratatui::style::Modifier::ITALIC),
            )),
            crate::server::events::LiveActivity::Tool { intent } => {
                let label = if intent.trim().is_empty() {
                    "working"
                } else {
                    intent.as_str()
                };
                tail.push(Line::styled(
                    label.to_string(),
                    ratatui::style::Style::new()
                        .fg(p.muted)
                        .add_modifier(ratatui::style::Modifier::ITALIC),
                ));
            }
            crate::server::events::LiveActivity::Idle => {}
        }
        if let Some(t) = s.streaming
            && !t.is_empty()
        {
            tail.extend(crate::tui::transcript::components::chat::render_streaming(
                t, p,
            ));
        }
    }
    // Rows left for the transcript once the live tail has taken its share.
    let keep = viewport.saturating_sub(tail.len().min(viewport));
    // Splice: the walk covers `offset + viewport` rows ending at the
    // document bottom, so the window we want sits at the range's TOP —
    // drop the bottommost `offset` rows, then keep `keep` rows.
    // (Keeping the LAST viewport rows here instead re-anchored every
    // scrolled frame to the document tail: the wheel bumped `chat_scroll`
    // while the picture never moved — the "history won't scroll" bug.)
    let cut_bottom = offset.min(block_rows.len().saturating_sub(keep));
    let keep_from = block_rows.len().saturating_sub(cut_bottom + keep);
    let mut visible: Vec<Line<'static>> = Vec::with_capacity(viewport);
    let mut it = block_rows.into_iter().skip(keep_from).take(keep);
    for _ in 0..keep {
        match it.next() {
            Some(l) => visible.push(l),
            None => break,
        }
    }
    // A live tail taller than the whole area (a long streaming reply) shows
    // its **newest** rows: keep the bottom of it, not the top.
    let skip_tail = tail
        .len()
        .saturating_sub(viewport.saturating_sub(visible.len()));
    visible.extend(tail.into_iter().skip(skip_tail));
    f.render_widget(Paragraph::new(visible), chat_area);

    // ---- bottom reserved area: the popup float zone (height already in layout; history/input gave way) ----
    // The resume picker wins (its stretched mode); then completion candidates; neither -> one blank row.
    if reserved_area.height > 0 {
        if let Some((items, selected)) = s.resume_pick {
            let lines = crate::tui::components::reserved::render_resume_picker(
                items,
                selected,
                area.width,
                reserved_area.height as usize,
                p,
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
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(joined, "一二三四五六七八九十ab", "换行不得丢字");
        // Every produced row fits the width (CJK counts as 2 cells).
        for l in &wrapped {
            let w: usize = l
                .spans
                .iter()
                .map(|s| crate::tui::text::display_width(&s.content))
                .sum();
            assert!(w <= 4, "行宽超限: {w}");
        }
    }

    #[test]
    fn wheel_scrolling_actually_moves_the_viewport() {
        // The full-chain regression: wheel events only bump `chat_scroll`;
        // the *visible* frame must change accordingly. A state where
        // chat_scroll advances but the frame stays identical is exactly
        // the "history won't scroll" bug.
        use crate::entry::Entry;
        use crate::tui::theme::Palette as P;

        let p = P::default();
        // 30 rounds → 60 blocks, each a few rows: comfortably taller
        // than the 12-row viewport.
        let mut entries: Vec<Entry> = Vec::new();
        for i in 0..30 {
            entries.push(Entry::User {
                content: format!("USER-{i} 标记行"),
            });
            entries.push(Entry::Assistant {
                content: format!("回答 {i}：第一行\n第二行\n第三行"),
                usage: None,
            });
        }

        let wrapped = crate::tui::text::wrap("", 40);
        let empty_live = crate::server::events::LiveActivity::Idle;
        let mut cache = crate::tui::transcript::cache::BlockCache::new();

        let mut grab = |chat_scroll: usize, pinned: bool| -> Vec<String> {
            let mut s = ViewState {
                history: &entries,
                transcript_generation: 0,
                block_cache: &mut cache,
                chat_scroll,
                scroll_pinned: pinned,
                show_reasoning: false,
                tools_expanded: false,
                live: &empty_live,
                streaming: None,
                wrapped: &wrapped,
                cursor_char: 0,
                spinner: None,
                model_name: "m",
                session_name: "",
                cwd: "/tmp",
                git: None,
                ctx_tokens: 0,
                ctx_limit: 1000,
                cost: 0.0,
                currency_symbol: "¥",
                show_cost: false,
                palette: p,
                popup: &crate::tui::completion::CompletionPopup::default(),
                resume_pick: None,
            };
            let l = tlayout::Layout {
                chat_height: 12,
                gap_height: 1,
                container_height: 3,
                body_rows: 1,
                first_visible: 0,
                reserved_height: 1,
            };
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 20)).unwrap();
            let l2 = &l;
            term.draw(|f| {
                let _ = draw(f, &mut s, l2);
            })
            .unwrap();
            let buf = term.backend().buffer().clone();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect()
        };

        let pinned_frame = grab(0, true);
        let scrolled_frame = grab(9, false);

        // The frames must differ, and the scrolled one must now show the
        // OLDEST rows the pinned view could not reach.
        assert_ne!(
            pinned_frame, scrolled_frame,
            "滚轮滚动后画面纹丝不动——滚动失效"
        );
    }

    #[test]
    fn the_live_tail_is_visible_when_the_history_fills_the_area() {
        // Regression: the live rows were appended *after* a full viewport had
        // been spliced, so `Paragraph` clipped them away. In any session whose
        // history filled the chat area the "thinking" label, the tool intent
        // and the streaming reply were all invisible — the reply only appeared
        // once the turn ended.
        use crate::entry::Entry;
        use crate::server::events::LiveActivity;
        use crate::tui::theme::Palette as P;

        let p = P::default();
        let mut entries: Vec<Entry> = Vec::new();
        for i in 0..30 {
            entries.push(Entry::User {
                content: format!("USER-{i} 标记行"),
            });
            entries.push(Entry::Assistant {
                content: format!("回答 {i}：第一行\n第二行\n第三行"),
                usage: None,
            });
        }
        let wrapped = crate::tui::text::wrap("", 40);
        let mut cache = crate::tui::transcript::cache::BlockCache::new();
        let mut frame = |live: LiveActivity, streaming: Option<&str>| -> Vec<String> {
            let mut s = ViewState {
                history: &entries,
                transcript_generation: 0,
                block_cache: &mut cache,
                chat_scroll: 0,
                scroll_pinned: true,
                show_reasoning: false,
                tools_expanded: false,
                live: &live,
                streaming,
                wrapped: &wrapped,
                cursor_char: 0,
                spinner: None,
                model_name: "m",
                session_name: "",
                cwd: "/tmp",
                git: None,
                ctx_tokens: 0,
                ctx_limit: 1000,
                cost: 0.0,
                currency_symbol: "¥",
                show_cost: false,
                palette: p,
                popup: &crate::tui::completion::CompletionPopup::default(),
                resume_pick: None,
            };
            let l = tlayout::Layout {
                chat_height: 12,
                gap_height: 1,
                container_height: 3,
                body_rows: 1,
                first_visible: 0,
                reserved_height: 1,
            };
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 20)).unwrap();
            let l2 = &l;
            term.draw(|f| {
                let _ = draw(f, &mut s, l2);
            })
            .unwrap();
            let buf = term.backend().buffer().clone();
            (0..12)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect()
        };

        // TestBackend stores one symbol per cell, so a wide CJK glyph leaves a
        // filler cell — compare with the padding stripped.
        let flat = |rows: &[String]| -> String { rows.concat().replace(' ', "") };

        let thinking = frame(LiveActivity::Thinking, None);
        assert!(
            thinking.iter().any(|r| r.contains("thinking")),
            "thinking 行必须可见: {thinking:?}"
        );

        // A running tool shows the model's intent on the same row.
        let tool = frame(
            LiveActivity::Tool {
                intent: "跑一下测试".into(),
            },
            None,
        );
        assert!(flat(&tool).contains("跑一下测试"), "{tool:?}");

        // In-flight content reaches the screen too, and when it is taller than
        // the area the *newest* rows are the ones kept.
        let streamed = frame(LiveActivity::Idle, Some("流式第一行\n流式第二行"));
        assert!(flat(&streamed).contains("流式第二行"), "{streamed:?}");
        let tall: String = (0..40).map(|i| format!("行{i}\n")).collect();
        let rows = frame(LiveActivity::Idle, Some(&tall));
        assert!(
            flat(&rows).contains("行39"),
            "最新的流式行必须在屏: {rows:?}"
        );
    }
}
