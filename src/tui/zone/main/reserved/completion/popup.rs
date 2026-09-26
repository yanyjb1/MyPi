//! Completion popup — the candidate list shown above the input container.
//!
//! Why above the container instead of overlaying the input:
//! an overlay would hide the text being typed — exactly what the user reads to pick a candidate.

use ratatui::text::Line;

use super::engine::CompletionPopup;
use crate::tui::theme::Palette;

// The popup's rendering result.
pub struct PopupView {
    // Rows, top to bottom (item 0 renders at the very top).
    //
    // Screen order, never reversed: ↑ moves up,
    // matching what the user sees — no "pressed up, jumped to the top" confusion.
    pub lines: Vec<Line<'static>>,
    // 弹窗左缘的终端绝对列：锚定在光标 x 下方；右缘溢出时整体左移
    // （右缘贴终端内边）。渲染方从这里拿偏移，不再自己算。
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

    // 锚定 + 左移规则（用户决策）：弹窗从光标 x 开始；内容宽度 +
    // anchor 超出终端右缘 → 整体左移，右缘贴终端内边，光标 x 与弹窗
    // 始终对应/靠近。
    let inner = crate::tui::zone::main::geometry::inner_width(term_w);
    let popup_w = visible
        .iter()
        .map(|(name, detail, _, _)| {
            2 + crate::tui::text::display_width(name)
                + if detail.is_empty() {
                    0
                } else {
                    2 + crate::tui::text::display_width(detail)
                }
        })
        .max()
        .unwrap_or(0)
        + 2; // 每行两格内边距（下方 line.push 两侧各 1 空格）
    let anchor_col = if anchor_col + popup_w > inner {
        inner.saturating_sub(popup_w)
    } else {
        anchor_col
    };
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible.len());

    for (name, detail, selected, is_dir) in visible {
        // Selected: accent `> ` prefix; unselected: gray spaces.
        // Name always accent; description (command detail / model display name) muted italic.
        let marker = if selected { "> " } else { "  " };
        let mut spans = Vec::new();
        if selected {
            spans.push(p.accent_span(marker));
        } else {
            spans.push(p.plain(marker));
        }
        spans.push(p.accent_span(name.to_string()));
        if !detail.is_empty() {
            spans.push(p.plain("  "));
            spans.push(p.muted_italic(detail.to_string()));
        }
        // Directory suffix hint (candidate names already carry `/`; the character suffices, no color coding)
        let _ = is_dir;
        // Pad to the popup's own width（不做全终端宽——弹窗贴着光标）。
        let used: usize = spans
            .iter()
            .map(|s| crate::tui::text::display_width(&s.content))
            .sum();
        let row_w = popup_w.saturating_sub(2); // 两格内边距已另计
        if used < row_w {
            spans.push(p.plain(" ".repeat(row_w - used)));
        }
        // 2 cells of padding on each side, aligned with the input container's border
        let mut line = Vec::with_capacity(spans.len() + 2);
        line.push(p.plain("  "));
        line.extend(spans);
        line.push(p.plain("  "));
        lines.push(Line::from(line));
    }

    // Pad blank placeholder rows up to the locked height
    let blank_inner = " ".repeat(popup_w.saturating_sub(2));
    while lines.len() < target {
        lines.push(Line::from(vec![
            p.plain("  "),
            p.plain(blank_inner.clone()),
            p.plain("  "),
        ]));
    }

    PopupView {
        lines,
        anchor_col,
    }
}

#[cfg(test)]
mod tests {
    use super::super::engine::{Completion, CompletionPopup};
    use super::*;

    fn cand(name: &str, is_dir: bool) -> Completion {
        Completion {
            name: name.to_string(),
            detail: String::new(),
            is_dir,
            insert: name.to_string(),
        }
    }

    fn text_of(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn closed_popup_renders_nothing() {
        let p = Palette::default();
        let popup = CompletionPopup::default();
        assert!(render(&popup, 80, 10, 0, &p).lines.is_empty());
    }

    #[test]
    fn lines_are_in_natural_order() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("alpha", false), cand("beta", false)], 0, 2);
        let v = render(&popup, 40, 10, 0, &p);
        assert_eq!(v.lines.len(), 2);
        // Keep screen order: the first entry on top
        assert!(text_of(&v.lines[0]).contains("alpha"));
        assert!(text_of(&v.lines[1]).contains("beta"));
    }

    #[test]
    fn selected_row_is_marked() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("alpha", false), cand("beta", false)], 0, 2);
        popup.move_selection(1); // 选 beta
        let v = render(&popup, 40, 10, 0, &p);
        assert!(text_of(&v.lines[1]).contains("> beta"));
        assert!(text_of(&v.lines[0]).contains("  alpha"));
    }

    #[test]
    fn row_width_is_content_not_full_terminal() {
        use unicode_width::UnicodeWidthStr;
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("a", false)], 0, 1);
        let v = render(&popup, 40, 10, 0, &p);
        let w: usize = v.lines[0].spans.iter().map(|s| s.content.width()).sum();
        // 新契约：弹窗贴光标，宽度是内容宽（> a 本身，含边距），不再撑满终端。
        assert!(w > 1, "行宽必须含边距");
        assert!(w < 40, "行宽 {w} 不该撑满终端 40 列");
    }

    #[test]
    fn anchor_shifts_left_on_overflow() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        let long = "y".repeat(50);
        popup.open(vec![cand(&long, false)], 0, 1);
        // 锚点 70 + 内容 52+2 > 40 列终端 → 左移，anchor = inner - popup_w
        let v = render(&popup, 40, 10, 70, &p);
        assert!(
            v.anchor_col + 52 <= 40 || v.anchor_col < 70,
            "溢出必须左移: anchor={}",
            v.anchor_col
        );
        assert_eq!(v.anchor_col, 0, "40 列终端放不下 52 列弹窗，贴最左");
    }

    #[test]
    fn very_long_name_does_not_break_alignment() {
        use unicode_width::UnicodeWidthStr;
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        let long = "x".repeat(200);
        popup.open(vec![cand(&long, false)], 0, 1);
        let v = render(&popup, 40, 10, 0, &p);
        let w: usize = v.lines[0].spans.iter().map(|s| s.content.width()).sum();
        assert!(w >= 40, "超长名字不 panic，宽度 {w}");
    }

    #[test]
    fn cjk_names_render_correctly() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("中文目录/", true)], 0, 1);
        let v = render(&popup, 40, 10, 0, &p);
        assert!(text_of(&v.lines[0]).contains("中文目录/"));
    }
}
