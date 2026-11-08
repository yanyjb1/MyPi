//! One name's versions, newest on top.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::tui::text::take_width;
use crate::ui::tui::theme::{Accent, fixed};

use super::{Face, MemoHand, first_line, window_start};

pub(super) fn body(
    hand: &MemoHand,
    width: u16,
    rows: usize,
    accent: &Accent,
) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let Face::History {
        scope,
        name,
        versions,
        selected,
        loaded,
    } = &hand.face
    else {
        return Vec::new();
    };
    let head = format!("{}/{} · {} 版", scope.as_str(), name, versions.len());
    let mut out = vec![Line::from(Span::styled(
        take_width(&head, width).0,
        Style::new().fg(fixed::MUTED),
    ))];
    if !*loaded {
        out.push(Line::from(Span::styled(
            "正在读取…",
            Style::new().fg(fixed::MUTED),
        )));
        return out;
    }
    let visible = rows.saturating_sub(1).max(1);
    let start = window_start(versions.len(), visible, *selected);
    for at in start..start + visible {
        let Some(index) = versions.len().checked_sub(1 + at) else {
            break;
        };
        let text = take_width(first_line(&versions[index].content), width).0;
        out.push(match at == *selected {
            true => Line::from(accent.span(text)),
            false => Line::from(Span::raw(text)),
        });
    }
    out
}
