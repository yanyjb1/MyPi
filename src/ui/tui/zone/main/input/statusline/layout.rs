//! One row of exactly `width` cells: groups, connectors, and whole-deco dropping.
//! The frame and the connectors belong to the engine, never to a deco.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::tui::zone::Ctx;
use unicode_width::UnicodeWidthStr;

use super::theme::{StatusTheme, Token};
use super::{Deco, Entry, PRIORITY_BASE, Side};

/// The row: table entries paired with what they built.
pub(super) type Rows = [(&'static Entry, Box<dyn Deco>)];

/// How much room the flexible decos get during one assembly pass.
#[derive(Debug, Clone, Copy)]
enum Flex {
    /// Measurement: flexible decos contribute their narrowest form.
    Min,
    /// Measurement of everything but the flexible bodies.
    Omit,
    /// Final assembly: this many cells are available to the flexible group.
    Exact(usize),
}

/// Render the whole row.
pub(super) fn render_line(rows: &Rows, t: &StatusTheme, width: u16, ctx: &Ctx) -> Line<'static> {
    let w = width as usize;
    let mut kept = vec![true; rows.len()];

    // Drop whole decos, lowest priority first, until the row fits.
    loop {
        if width_of(&assemble(rows, &kept, t, Flex::Min, ctx)) <= w {
            break;
        }
        match victim(rows, &kept) {
            Some(i) => kept[i] = false,
            None => break, // only the base tier is left; `fit_width` clamps
        }
    }

    let fixed = width_of(&assemble(rows, &kept, t, Flex::Omit, ctx));
    let spans = assemble(rows, &kept, t, Flex::Exact(w.saturating_sub(fixed)), ctx);
    Line::from(fit_width(spans, w))
}

/// The lowest-priority kept deco that may disappear. Base-tier decos never are;
/// ties keep the earlier one.
fn victim(rows: &Rows, kept: &[bool]) -> Option<usize> {
    let mut best: Option<(u8, usize)> = None;
    for (i, (entry, _)) in rows.iter().enumerate() {
        if !kept[i] || entry.priority == PRIORITY_BASE {
            continue;
        }
        match best {
            Some((p, _)) if p <= entry.priority => {}
            _ => best = Some((entry.priority, i)),
        }
    }
    best.map(|(_, i)| i)
}

/// Lay the row out. Decos are visited in `(side, order)` order.
fn assemble(
    rows: &Rows,
    kept: &[bool],
    t: &StatusTheme,
    flex: Flex,
    ctx: &Ctx,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    out.push(t.fg("+", Token::Accent));
    out.push(t.fg("--", Token::Accent));
    out.push(t.on_capsule(" "));

    // Left group: ` > ` between neighbours, nothing if a deco stays empty.
    let mut any_left = false;
    for (i, (entry, deco)) in rows.iter().enumerate() {
        if !kept[i] || entry.side != Side::Left {
            continue;
        }
        let spans = deco.render(t, None, ctx);
        if spans.is_empty() {
            continue;
        }
        if any_left {
            out.push(t.sep(" > "));
        }
        out.extend(spans);
        any_left = true;
    }

    // Flexible group: the lead space, then the deco's own connector.
    let flexes: Vec<&Box<dyn Deco>> = rows
        .iter()
        .enumerate()
        .filter(|(i, (entry, _))| kept[*i] && entry.side == Side::Flex)
        .map(|(_, (_, deco))| deco)
        .collect();
    if !flexes.is_empty() {
        // The lead space counts as overhead, so it is emitted in `Omit` mode too.
        if any_left {
            out.push(t.on_capsule(" "));
        }
        if !matches!(flex, Flex::Omit) {
            let mut remaining = match flex {
                Flex::Exact(n) => n,
                _ => 0,
            };
            for c in flexes {
                let spans = match flex {
                    Flex::Exact(_) => {
                        let s = c.render(t, Some(remaining), ctx);
                        remaining = remaining.saturating_sub(width_of(&s));
                        s
                    }
                    _ => c.render(t, None, ctx),
                };
                out.extend(spans);
            }
        }
    }

    // Right group: ` | ` boundary, then every deco prefixed by `< `.
    let rights: Vec<&Box<dyn Deco>> = rows
        .iter()
        .enumerate()
        .filter(|(i, (entry, _))| kept[*i] && entry.side == Side::Right)
        .map(|(_, (_, deco))| deco)
        .collect();
    let mut any_right = false;
    for c in rights {
        let spans = c.render(t, None, ctx);
        if spans.is_empty() {
            continue;
        }
        if !any_right {
            out.push(t.fg(" | ", Token::Accent));
        }
        out.push(t.fg("<", Token::Accent));
        out.push(t.on_capsule(" "));
        out.extend(spans);
        any_right = true;
    }

    out.push(t.on_capsule(" "));
    out.push(t.fg("--", Token::Accent));
    out.push(t.fg("+", Token::Accent));
    out
}

/// Display width of a span run.
fn width_of(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Truncate by display width, pad with plain spaces: the row is `w` cells.
fn fit_width(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    let used = width_of(&spans);
    if used <= w {
        let mut out = spans;
        out.push(Span::styled(" ".repeat(w - used), Style::new()));
        return out;
    }
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut acc = 0usize;
    for s in spans {
        let sw = s.content.width();
        if acc + sw <= w {
            acc += sw;
            out.push(s);
        } else {
            let mut kept = String::new();
            for ch in s.content.chars() {
                let cw = ch.to_string().width();
                if acc + cw > w {
                    break;
                }
                kept.push(ch);
                acc += cw;
            }
            out.push(Span::styled(kept, s.style));
            break;
        }
    }
    out
}
