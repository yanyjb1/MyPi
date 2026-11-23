// One version, whole: every line, wrapped to the panel.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::tui::theme::{Accent, fixed};

use super::{Face, MemoHand, clip, layout, wrap};

pub(super) fn body(
    hand: &MemoHand,
    width: u16,
    rows: usize,
    accent: &Accent,
) -> Vec<Line<'static>> {
    let Face::Read {
        scope,
        name,
        versions,
        selected,
        row,
    } = &hand.face
    else {
        return Vec::new();
    };
    let Some(version) = versions.iter().rev().nth(*selected) else {
        return Vec::new();
    };
    let (block, lead) = layout(usize::from(width), hand.narrow());
    let lead = " ".repeat(lead);
    let head = match &version.ts {
        Some(ts) => format!("{}/{} · {ts}", scope.as_str(), name),
        None => format!("{}/{}", scope.as_str(), name),
    };
    let mut out = vec![Line::from(Span::styled(
        format!("{lead}{}", clip(&head, block)),
        Style::new().fg(fixed::MUTED),
    ))];
    let visible = rows.saturating_sub(1).max(1);
    let mut visual: Vec<(usize, String)> = Vec::new();
    for (at, line) in version.content.lines().enumerate() {
        if line.is_empty() {
            visual.push((at, String::new()));
        } else {
            for piece in wrap(line, block) {
                visual.push((at, piece));
            }
        }
    }
    let start = visual.iter().position(|(at, _)| at == row).unwrap_or(0);
    for (at, text) in visual.into_iter().skip(start).take(visible) {
        let line = format!("{lead}{text}");
        out.push(match at == *row {
            true => Line::from(accent.span(line)),
            false => Line::from(Span::raw(line)),
        });
    }
    out
}
