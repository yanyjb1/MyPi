// The name list: scope, name, and the first line of the newest text.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::tui::lang::Lang;
use crate::ui::tui::text::display_width;
use crate::ui::tui::theme::{Accent, fixed};

use super::super::grid::Grid;
use super::{MemoHand, clip, first_line, layout, window_start};

// The scope column: `current` and `global` are both seven cells.
const SCOPE: usize = 7;

const NAME_MIN: usize = 4;
const NAME_MAX: usize = 24;

pub(super) fn body(
    hand: &MemoHand,
    width: u16,
    rows: usize,
    accent: &Accent,
) -> Vec<Line<'static>> {
    if !hand.known {
        return Vec::new();
    }
    let width = usize::from(width);
    let (block, lead) = layout(width, hand.narrow());
    if block < SCOPE + NAME_MIN + 3 {
        let word = match hand.lang {
            Lang::En => "window too narrow",
            Lang::Zh => "窗口太窄",
        };
        return vec![Line::from(clip(word, width))];
    }
    let name_w = hand
        .items
        .iter()
        .map(|item| display_width(&item.name))
        .max()
        .unwrap_or(NAME_MIN)
        .clamp(NAME_MIN, NAME_MAX)
        .min(block - SCOPE - 3);
    let rest = block.saturating_sub(SCOPE + name_w + 2);
    let widths = [SCOPE, name_w, rest];
    let grid = Grid {
        widths: &widths,
        right: &[false, false, false],
        gap: 1,
        lead,
    };
    let head = grid.head(&[
        "scope".to_string(),
        "name".to_string(),
        "content".to_string(),
    ]);
    let mut out = vec![head];
    if hand.items.is_empty() {
        let word = match hand.lang {
            Lang::En => "no memos",
            Lang::Zh => "没有备忘录",
        };
        out.push(Line::from(Span::styled(
            format!("{}{word}", " ".repeat(lead)),
            Style::new().fg(fixed::MUTED),
        )));
        return out;
    }
    let visible = rows.saturating_sub(1).max(1);
    let start = window_start(hand.items.len(), visible, hand.selected);
    for (at, item) in hand.items.iter().enumerate().skip(start).take(visible) {
        let cells = [
            item.scope.as_str().to_string(),
            item.name.clone(),
            first_line(&item.content).to_string(),
        ];
        out.push(grid.row(&cells, at == hand.selected, accent));
    }
    out
}
