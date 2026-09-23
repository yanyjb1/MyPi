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
}

// Render the popup. Empty content returns nothing.
//
// `avail` is the popup's row capacity (given by the caller from the space above the container).
pub fn render(popup: &CompletionPopup, term_w: u16, avail: usize, p: &Palette) -> PopupView {
    let visible = popup.visible(avail);
    if visible.is_empty() {
        return PopupView { lines: Vec::new() };
    }

    // Locked height: pad with blank rows when candidates shrink; the reserved area stays put (no jitter)
    let target = popup.locked_height().unwrap_or(visible.len());

    // The border width uses the project's single definition; never hand-write 4 again:
    // the last sweep fixed 5 hardcoded sites; this was the 6th that slipped through.
    let inner = crate::tui::layout::inner_width(term_w);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible.len());

    for (name, detail, selected, is_dir) in visible {
        // Selected: accent `> ` prefix; unselected: gray spaces.
        // Name always accent; description (command detail / model display name) muted italic.
        let marker = if selected { "> " } else { "  " };
        let mut spans = Vec::new();
        if selected {
            spans.push(p.accent(marker));
        } else {
            spans.push(p.plain(marker));
        }
        spans.push(p.accent(name.to_string()));
        if !detail.is_empty() {
            spans.push(p.plain("  "));
            spans.push(p.muted_italic(detail.to_string()));
        }
        // Directory suffix hint (candidate names already carry `/`; the character suffices, no color coding)
        let _ = is_dir;
        // Pad to the full row width so no stale terminal background remains
        let used: usize = spans.iter().map(|s| crate::tui::text::display_width(&s.content)).sum();
        if used < inner {
            spans.push(p.plain(" ".repeat(inner - used)));
        }
        // 2 cells of padding on each side, aligned with the input container's border
        let mut line = Vec::with_capacity(spans.len() + 2);
        line.push(p.plain("  "));
        line.extend(spans);
        line.push(p.plain("  "));
        lines.push(Line::from(line));
    }

    // Pad blank placeholder rows up to the locked height
    let blank_inner = " ".repeat(inner);
    while lines.len() < target {
        lines.push(Line::from(vec![
            p.plain("  "),
            p.plain(blank_inner.clone()),
            p.plain("  "),
        ]));
    }

    PopupView { lines }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::engine::{Completion, CompletionPopup};

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
        assert!(render(&popup, 80, 10, &p).lines.is_empty());
    }

    #[test]
    fn lines_are_in_natural_order() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("alpha", false), cand("beta", false)], 0, 2);
        let v = render(&popup, 40, 10, &p);
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
        let v = render(&popup, 40, 10, &p);
        assert!(text_of(&v.lines[1]).contains("> beta"));
        assert!(text_of(&v.lines[0]).contains("  alpha"));
    }

    #[test]
    fn row_width_equals_terminal_width() {
        use unicode_width::UnicodeWidthStr;
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("a", false)], 0, 1);
        let v = render(&popup, 40, 10, &p);
        let w: usize = v.lines[0].spans.iter().map(|s| s.content.width()).sum();
        assert_eq!(w, 40);
    }

    #[test]
    fn very_long_name_does_not_break_alignment() {
        use unicode_width::UnicodeWidthStr;
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        let long = "x".repeat(200);
        popup.open(vec![cand(&long, false)], 0, 1);
        let v = render(&popup, 40, 10, &p);
        let w: usize = v.lines[0].spans.iter().map(|s| s.content.width()).sum();
        assert!(w >= 40, "超长名字不 panic，宽度 {w}");
    }

    #[test]
    fn cjk_names_render_correctly() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("中文目录/", true)], 0, 1);
        let v = render(&popup, 40, 10, &p);
        assert!(text_of(&v.lines[0]).contains("中文目录/"));
    }
}
