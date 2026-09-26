//! The width engine — one row, exactly `width` cells, no exceptions.
//!
//! Order of operations:
//!
//! 1. The frame (`+--` … `--+`) and the connectors (` > `, ` | `, `< `) belong
//!    to the engine, never to a component: dropping a component can never
//!    leave a dangling connector behind.
//! 2. Flexible components ([`Side::Flex`]) are the slack absorbers. They take
//!    what is left once everything else is placed, and their **narrowest**
//!    form counts as part of the base tier — so the gauge can never be pushed
//!    out by a long model name.
//! 3. When even that does not fit, whole components are dropped, lowest
//!    priority first, connector included. Text is never cut mid-way.
//! 4. [`fit_width`] is the last-resort guard: the statusline is row 0 of the
//!    input container and its width **is** the container's width, so a row
//!    that is one cell off tears every row below it.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::theme::{StatusTheme, Token};
use super::{PRIORITY_BASE, Side, StatusComponent};

/// How much room the flexible components get during one assembly pass.
#[derive(Debug, Clone, Copy)]
enum Flex {
    /// Measurement: flexible components contribute their narrowest form.
    Min,
    /// Measurement of everything *but* the flexible bodies.
    Omit,
    /// Final assembly: this many cells are available to the flexible group.
    Exact(usize),
}

/// Render the whole row.
pub(super) fn render_line(
    comps: &[Box<dyn StatusComponent>],
    t: &StatusTheme,
    width: u16,
) -> Line<'static> {
    let w = width as usize;
    // A component may also render nothing on its own (the cost segment without
    // a price sheet): `assemble` skips it, connector included.
    let mut kept = vec![true; comps.len()];

    // Drop whole components until the row fits. Each pass is one render of the
    // registry (seven components), and there are at most `comps.len()` passes.
    loop {
        if width_of(&assemble(comps, &kept, t, Flex::Min)) <= w {
            break;
        }
        match victim(comps, &kept) {
            Some(i) => kept[i] = false,
            None => break, // only the base tier is left; `fit_width` clamps
        }
    }

    let fixed = width_of(&assemble(comps, &kept, t, Flex::Omit));
    let spans = assemble(comps, &kept, t, Flex::Exact(w.saturating_sub(fixed)));
    Line::from(fit_width(spans, w))
}

/// The lowest-priority kept component that is allowed to disappear.
///
/// Base-tier components ([`PRIORITY_BASE`]) are never candidates: π, the price
/// and the gauge's narrowest form are the row's floor. Ties keep the earlier
/// component (registration order).
fn victim(comps: &[Box<dyn StatusComponent>], kept: &[bool]) -> Option<usize> {
    let mut best: Option<(u8, usize)> = None;
    for (i, c) in comps.iter().enumerate() {
        if !kept[i] || c.priority() == PRIORITY_BASE {
            continue;
        }
        match best {
            Some((p, _)) if p <= c.priority() => {}
            _ => best = Some((c.priority(), i)),
        }
    }
    best.map(|(_, i)| i)
}

/// Lay the row out. Components are visited in registry order, which
/// `StatusLine::new` keeps sorted by `(side, order)`.
fn assemble(
    comps: &[Box<dyn StatusComponent>],
    kept: &[bool],
    t: &StatusTheme,
    flex: Flex,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    // Frame open: accent, transparent background.
    out.push(t.fg("+", Token::Accent));
    out.push(t.fg("--", Token::Accent));
    out.push(t.on_capsule(" "));

    // Left group: ` > ` between neighbours, nothing if a component stays empty.
    let mut any_left = false;
    for (i, c) in comps.iter().enumerate() {
        if !kept[i] || c.side() != Side::Left {
            continue;
        }
        let spans = c.render(t, None);
        if spans.is_empty() {
            continue;
        }
        if any_left {
            out.push(t.sep(" > "));
        }
        out.extend(spans);
        any_left = true;
    }

    // Flexible group: one space off the left group, then the component's own
    // connector (`>`), which is why the engine adds no `>` of its own.
    let flexes: Vec<&Box<dyn StatusComponent>> = comps
        .iter()
        .enumerate()
        .filter(|(i, c)| kept[*i] && c.side() == Side::Flex)
        .map(|(_, c)| c)
        .collect();
    if !flexes.is_empty() {
        // The lead space belongs to the flexible group's overhead, so it is
        // emitted even in `Omit` mode — otherwise the budget handed to the
        // bodies would be one cell too generous and the row would overshoot.
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
                        let s = c.render(t, Some(remaining));
                        remaining = remaining.saturating_sub(width_of(&s));
                        s
                    }
                    _ => c.render(t, None),
                };
                out.extend(spans);
            }
        }
    }

    // Right group: ` | ` boundary, then every component prefixed by `< `.
    let rights: Vec<&Box<dyn StatusComponent>> = comps
        .iter()
        .enumerate()
        .filter(|(i, c)| kept[*i] && c.side() == Side::Right)
        .map(|(_, c)| c)
        .collect();
    let mut any_right = false;
    for c in rights {
        let spans = c.render(t, None);
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

    // Frame close: capsule padding, then accent.
    out.push(t.on_capsule(" "));
    out.push(t.fg("--", Token::Accent));
    out.push(t.fg("+", Token::Accent));
    out
}

/// Display width of a span run.
fn width_of(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Last-resort guard: hard-truncate by display width when still too wide, pad
/// with plain spaces when short. Nothing else in the row may violate the width.
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
