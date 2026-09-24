//! Bottom reserved area — rows fixed at the very bottom of the terminal.
//!
//! Key point: it is **independent blank rows**, not part of the input container.
//! This draws nothing — no borders, no characters, just blank rows.
//! Drawing `+- ... -+` was wrong: it read as a second bottom edge of the editor,
//! visually attaching itself to the input box.
//!
//! Height comes from the layout. Zero disables the area and returns the rows
//! to the chat area.

use ratatui::text::Line;

// Default height cap when the reserved area floats up.
//
// Callers (commands/completion/services) may declare a larger or smaller max; this is the fallback.
pub const DEFAULT_MAX: usize = 6;

// The /resume session picker: title row + session list (highlight `> `).
//
// Overflowing tails are truncated (newest first; the top entries matter most).
pub fn render_resume_picker(
    items: &[(i64, String)],
    selected: usize,
    term_w: u16,
    height: usize,
    p: &crate::tui::theme::Palette,
) -> Vec<Line<'static>> {
    let inner = crate::tui::layout::inner_width(term_w).saturating_sub(4);
    let mut out = vec![Line::from(vec![
        p.plain("  "),
        p.accent_span("选择要恢复的会话（↑↓ 移动，Enter 恢复，Esc 取消）"),
    ])];
    // Session rows that fit = height - title
    let rows = height.saturating_sub(1).min(items.len().max(1));
    // Scrolling window: keep the selection visible
    let start = if selected < rows {
        0
    } else {
        selected + 1 - rows
    };
    for (i, (_, name)) in items.iter().enumerate().skip(start).take(rows) {
        let marker = if i == selected { "> " } else { "  " };
        let mut spans = Vec::new();
        if i == selected {
            spans.push(p.accent_span(marker));
            spans.push(p.accent_span(name.clone()));
        } else {
            spans.push(p.plain(marker));
            spans.push(p.muted_span(name.clone()));
        }
        // Truncate names that are too wide
        let used: usize = spans
            .iter()
            .map(|s| crate::tui::text::display_width(&s.content))
            .sum();
        if used > inner {
            spans.truncate(2);
            spans[1] = p.plain(crate::tui::text::take_width(name, inner.saturating_sub(1)).0);
        }
        out.push(Line::from(spans));
    }
    out
}

// Render the reserved area. Empty `lines` -> one blank row (idle);
// non-empty -> passed through (candidate rows come from the caller; this only claims the space).
pub fn render<'a>(lines: Vec<Line<'a>>) -> Vec<Line<'a>> {
    if lines.is_empty() {
        vec![Line::from("")]
    } else {
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn text_of(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn idle_is_one_blank_line() {
        let l = render(Vec::new());
        assert_eq!(l.len(), 1, "空闲态恰好一行");
        assert_eq!(text_of(&l[0]), "", "保留区必须是一个纯空行");
        let w: usize = l[0].spans.iter().map(|s| s.content.width()).sum();
        assert_eq!(w, 0, "不该有任何可见字符");
    }

    #[test]
    fn has_no_border_characters() {
        // Regression: it once drew `+-  -+`, looking like a second editor bottom edge
        for l in render(Vec::new()) {
            let t = text_of(&l);
            assert!(!t.contains('+'), "不该有边框角: {t:?}");
            assert!(!t.contains('-'), "不该有横线: {t:?}");
            assert!(!t.contains('|'), "不该有竖线: {t:?}");
        }
    }

    #[test]
    fn passthrough_keeps_caller_lines() {
        let l = render(vec![Line::from("候选1"), Line::from("候选2")]);
        assert_eq!(l.len(), 2, "非空内容原样透传");
        assert_eq!(text_of(&l[0]), "候选1");
    }
}
