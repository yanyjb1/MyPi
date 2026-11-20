//! One name's versions, newest on top, with the time each was written.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::tui::text::display_width;
use crate::ui::tui::theme::{Accent, fixed};

use super::super::grid::Grid;
use super::{Face, MemoHand, NARROW_WIDE, clip, first_line, layout, window_start};

pub(super) fn body(
    hand: &MemoHand,
    width: u16,
    rows: usize,
    accent: &Accent,
) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let (block, lead) = layout(width, NARROW_WIDE);
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
    let head = clip(
        &format!("{}/{} · {} 版", scope.as_str(), name, versions.len()),
        block,
    );
    let mut out = vec![Line::from(Span::styled(
        format!("{}{head}", " ".repeat(lead)),
        Style::new().fg(fixed::MUTED),
    ))];
    if !*loaded {
        out.push(Line::from(Span::styled(
            format!("{}正在读取…", " ".repeat(lead)),
            Style::new().fg(fixed::MUTED),
        )));
        return out;
    }
    let stamp_w = versions
        .iter()
        .filter_map(|version| version.ts.as_deref())
        .map(display_width)
        .max()
        .unwrap_or(0)
        .min(block / 2);
    let rest = block.saturating_sub(stamp_w + 1);
    let widths = [stamp_w, rest];
    let grid = Grid {
        widths: &widths,
        right: &[false, false],
        gap: 1,
        lead,
    };
    let visible = rows.saturating_sub(1).max(1);
    let start = window_start(versions.len(), visible, *selected);
    for at in start..start + visible {
        let Some(index) = versions.len().checked_sub(1 + at) else {
            break;
        };
        let version = &versions[index];
        let stamp = version.ts.clone().unwrap_or_else(|| " ".repeat(stamp_w));
        let cells = [stamp, first_line(&version.content).to_string()];
        out.push(grid.row(&cells, at == *selected, accent));
    }
    out
}
