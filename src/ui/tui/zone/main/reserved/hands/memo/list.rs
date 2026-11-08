//! The name list: scope, name, and the first line of the newest text.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::tui::text::{display_width, take_width};
use crate::ui::tui::theme::{Accent, fixed};

use super::{MemoHand, first_line, window_start};

/// The scope column: `current` and `global` are both seven cells.
const SCOPE: usize = 7;

const NAME_MIN: usize = 4;
const NAME_MAX: usize = 24;

pub(super) fn body(
    hand: &MemoHand,
    width: u16,
    rows: usize,
    accent: &Accent,
) -> Vec<Line<'static>> {
    let width = usize::from(width);
    if width < SCOPE + NAME_MIN + 3 {
        return vec![Line::from(clip("窗口太窄", width))];
    }
    let name_w = hand
        .items
        .iter()
        .map(|item| display_width(&item.name))
        .max()
        .unwrap_or(NAME_MIN)
        .clamp(NAME_MIN, NAME_MAX)
        .min(width - SCOPE - 3);
    let rest = width.saturating_sub(SCOPE + name_w + 2);
    let head = format!(
        "{} {} {}",
        pad("scope", SCOPE),
        pad("name", name_w),
        clip("content", rest)
    );
    let mut out = vec![Line::from(Span::styled(
        head,
        Style::new().fg(fixed::MUTED),
    ))];
    if hand.items.is_empty() {
        out.push(Line::from(Span::styled(
            "没有备忘录",
            Style::new().fg(fixed::MUTED),
        )));
        return out;
    }
    let visible = rows.saturating_sub(1).max(1);
    let start = window_start(hand.items.len(), visible, hand.selected);
    for (at, item) in hand.items.iter().enumerate().skip(start).take(visible) {
        let row = format!(
            "{} {} {}",
            pad(item.scope.as_str(), SCOPE),
            pad(&item.name, name_w),
            clip(first_line(&item.content), rest)
        );
        out.push(match at == hand.selected {
            true => Line::from(accent.span(row)),
            false => Line::from(Span::raw(row)),
        });
    }
    out
}

/// One cell, cut to `width` cells with a marker when it does not fit.
fn clip(text: &str, width: usize) -> String {
    let (head, used) = take_width(text, width);
    if used >= display_width(text) || width == 0 {
        return head;
    }
    let (head, _) = take_width(text, width - 1);
    format!("{head}…")
}

/// One cell, padded to `width` cells.
fn pad(text: &str, width: usize) -> String {
    let text = clip(text, width);
    let used = display_width(&text);
    format!("{text}{}", " ".repeat(width.saturating_sub(used)))
}
