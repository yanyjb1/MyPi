//! Row helpers: wrapping by display width, the gap between two visible slots.

use ratatui::text::{Line, Span};

use crate::ui::tui::text::display_width;

/// Wrap one logical line to `width` cells.
pub(crate) fn wrap_rows(
    lines: impl Iterator<Item = Line<'static>>,
    width: usize,
) -> Vec<Line<'static>> {
    let w = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        let total: usize = line.spans.iter().map(|s| display_width(&s.content)).sum();
        if total <= w {
            out.push(line);
            continue;
        }
        let mut cur: Vec<Span<'static>> = Vec::new();
        let mut cur_w = 0usize;
        for sp in line.spans {
            let mut buf = String::new();
            for ch in sp.content.chars() {
                let cw = display_width(&ch.to_string());
                if cur_w + cw > w && (cur_w > 0 || !buf.is_empty()) {
                    if !buf.is_empty() {
                        cur.push(Span::styled(std::mem::take(&mut buf), sp.style));
                    }
                    out.push(Line::from(std::mem::take(&mut cur)));
                    cur_w = 0;
                }
                buf.push(ch);
                cur_w += cw;
            }
            if !buf.is_empty() {
                cur.push(Span::styled(buf, sp.style));
            }
        }
        if !cur.is_empty() {
            out.push(Line::from(cur));
        }
    }
    out
}

/// One blank row between two visible slots.
pub(crate) fn block_gap() -> Line<'static> {
    Line::from("")
}
