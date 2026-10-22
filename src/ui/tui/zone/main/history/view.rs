//! The viewport: the sight line, the rows laid out around it, and when more is asked for.

use std::time::{Duration, Instant};

use ratatui::text::Line;

use super::cache::{BlockCache, Item};
use super::local::{Local, SlotKind};
use super::render::cards::notice::notice_block;
use super::render::theme::HistoryTheme;

/// The seam the sight line sits on: above row `row` of slot `num`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub num: i64,
    pub row: usize,
}

/// Which way the reader moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Older,
    Newer,
}

/// What one direction holds: blocks already painted, and whether the walk stopped at one
/// that is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ahead {
    pub painted: usize,
    pub more: bool,
}

/// The visible region.
pub struct Viewport {
    /// The sight line: rows counted from the top of the zone.
    line: u16,
    /// The seam the sight line sits on.
    anchor: Anchor,
    /// The newest block sits against the pinned rows.
    bottom: bool,
    /// Rows the main zone handed down.
    rows: u16,
    /// When the reader last jumped, if a receipt is still owed. Only a jump arms this.
    moved: Option<Instant>,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            line: 3,
            anchor: Anchor { num: 0, row: 0 },
            bottom: true,
            rows: 0,
            moved: None,
        }
    }
}

impl Viewport {
    pub fn set_line(&mut self, line: u16) {
        self.line = line.max(1);
    }

    pub fn assign(&mut self, rows: u16) {
        self.rows = rows;
    }

    /// The slot the sight line sits on.
    pub fn anchor(&self) -> i64 {
        self.anchor.num
    }

    /// Put a slot right under the sight line.
    pub fn focus_num(&mut self, num: i64) {
        self.anchor = Anchor { num, row: 0 };
        self.bottom = false;
    }

    /// One slot older or newer. The bottom's seam is one past the newest slot, so one step
    /// older lands on the newest; one step newer stays at the bottom.
    pub fn step(&mut self, local: &Local, dir: Dir) {
        if self.bottom {
            let seam = local.newest().map(|n| n + 1).unwrap_or(0);
            self.anchor = Anchor { num: seam, row: 0 };
            if dir == Dir::Newer {
                return;
            }
            self.bottom = false;
        }
        let next = match dir {
            Dir::Older => self.anchor.num - 1,
            Dir::Newer => self.anchor.num + 1,
        };
        if local.at(next).is_some() {
            self.anchor = Anchor { num: next, row: 0 };
        }
        self.restart();
    }

    /// Follow the newest block again.
    pub fn follow(&mut self) {
        self.bottom = true;
    }

    /// The oldest slot under the sight line.
    pub fn first(&mut self, local: &Local) {
        if let Some(num) = local.oldest() {
            self.anchor = Anchor { num, row: 0 };
            self.bottom = false;
            self.restart();
        }
    }

    /// A screen older or newer: whole slots until the rows are covered.
    pub fn page(&mut self, local: &Local, items: &[Item<'_>], cache: &BlockCache, dir: Dir) {
        let budget = usize::from(self.rows.max(1));
        let step = match dir {
            Dir::Older => -1,
            Dir::Newer => 1,
        };
        let mut covered = 0usize;
        let mut num = self.anchor.num;
        while covered < budget {
            num += step;
            let Some(slot) = local.at(num) else { break };
            covered += slot_height(slot, items, cache) + 1;
        }
        if local.at(num).is_some() {
            self.anchor = Anchor { num, row: 0 };
            self.bottom = false;
            self.restart();
        }
    }

    /// A jump went out: a receipt is owed once the reader has been still long enough.
    pub fn arm(&mut self) {
        self.moved = Some(Instant::now());
    }

    /// Any move restarts the count, so only a still reader is answered.
    fn restart(&mut self) {
        if self.moved.is_some() {
            self.moved = Some(Instant::now());
        }
    }

    /// The reader has been still long enough: a receipt is owed.
    pub fn note_still(&mut self, silence: Duration) -> bool {
        let Some(at) = self.moved else { return false };
        if at.elapsed() < silence {
            return false;
        }
        self.moved = None;
        true
    }

    /// A resize keeps the seam: the same slot, the same row inside it.
    pub fn on_resize(&mut self, local: &Local, items: &[Item<'_>], cache: &BlockCache) {
        if self.bottom {
            return;
        }
        let Some(slot) = local.at(self.anchor.num) else {
            self.anchor = Anchor {
                num: local.newest().unwrap_or(0),
                row: 0,
            };
            return;
        };
        let height = match slot.kind {
            SlotKind::Block(_) => item_of(items, slot.num)
                .and_then(|it| cache.known_height(slot.num, it.block))
                .unwrap_or(0),
            SlotKind::Next => 0,
            _ => 1,
        };
        self.anchor.row = self.anchor.row.min(height.saturating_sub(1));
    }

    /// Lay this frame's rows out: above the sight line, below it, then the pinned rows.
    pub fn rows(
        &mut self,
        local: &Local,
        items: &[Item<'_>],
        cache: &mut BlockCache,
        pinned: &[Line<'static>],
        tail: &[Line<'static>],
        skip: Option<i64>,
        t: &HistoryTheme,
        width: usize,
    ) -> Vec<Line<'static>> {
        let body = usize::from(self.rows).saturating_sub(pinned.len());
        if body == 0 {
            return pinned.to_vec();
        }
        let above = if self.bottom {
            body
        } else {
            usize::from(self.line).min(body)
        };
        let seam = if self.bottom {
            let seam = Anchor {
                num: local.newest().map(|n| n + 1).unwrap_or(0),
                row: 0,
            };
            self.anchor = seam;
            seam
        } else {
            self.anchor
        };

        let mut out: Vec<Line<'static>> = Vec::with_capacity(body);

        // Above the seam: the seam slot's own leading rows, then the slots before it.
        let mut head: Vec<Line<'static>> = Vec::new();
        if let Some(slot) = local.at(seam.num) {
            let rows = slot_rows(slot, items, cache, t, width);
            head.extend(rows.into_iter().take(seam.row));
        }
        let mut num = seam.num - 1;
        let mut prev_had_rows = !head.is_empty();
        while head.len() < above {
            let Some(slot) = local.at(num) else { break };
            if Some(slot.num) == skip {
                num -= 1;
                continue;
            }
            let rows = slot_rows(slot, items, cache, t, width);
            if !rows.is_empty() && prev_had_rows {
                head.push(Line::default());
            }
            prev_had_rows = !rows.is_empty();
            head.extend(rows);
            num -= 1;
        }
        let drop = head.len().saturating_sub(above);
        out.extend(head.into_iter().skip(drop));
        out.truncate(above);

        // Below the seam: the seam slot's remaining rows, then the slots after it.
        let mut wrote_any = !out.is_empty();
        if let Some(slot) = local.at(seam.num) {
            let rows = slot_rows(slot, items, cache, t, width);
            let tail: Vec<Line<'static>> = rows.into_iter().skip(seam.row).collect();
            if !tail.is_empty() && wrote_any {
                out.push(Line::default());
            }
            wrote_any |= !tail.is_empty();
            out.extend(tail);
        }
        let mut num = seam.num + 1;
        while out.len() < body {
            let Some(slot) = local.at(num) else {
                break;
            };
            if Some(slot.num) == skip {
                num += 1;
                continue;
            }
            let rows = slot_rows(slot, items, cache, t, width);
            if !rows.is_empty() && wrote_any {
                out.push(Line::default());
            }
            wrote_any |= !rows.is_empty();
            out.extend(rows);
            num += 1;
        }
        // Past the last slot: what a running tool has printed so far.
        if local.at(num).is_none() && !tail.is_empty() {
            if wrote_any {
                out.push(Line::default());
            }
            out.extend(tail.iter().cloned());
        }

        // Short of rows: the conversation ended. Pad so the pinned rows keep their place.
        out.truncate(body);
        while out.len() < body {
            out.push(Line::default());
        }
        out.extend(pinned.iter().cloned());
        out
    }

    /// Blocks already carrying rows in this direction, up to the first one without.
    pub fn overscan(&self, local: &Local, cache: &BlockCache, dir: Dir) -> Ahead {
        let step = match dir {
            Dir::Older => -1,
            Dir::Newer => 1,
        };
        let mut num = self.anchor.num;
        let mut painted = 0usize;
        for _ in 0..local.len() {
            num += step;
            let Some(slot) = local.at(num) else { break };
            if !matches!(slot.kind, SlotKind::Block(_)) {
                continue;
            }
            if !cache.has_rows(slot.num) {
                return Ahead {
                    painted,
                    more: true,
                };
            }
            painted += 1;
        }
        Ahead {
            painted,
            more: false,
        }
    }
}

/// The item for a number. The list is ascending, so this is a lookup, not a scan.
fn item_of<'a, 'b>(items: &'b [Item<'a>], num: i64) -> Option<&'b Item<'a>> {
    items
        .binary_search_by_key(&num, |it| it.num)
        .ok()
        .map(|i| &items[i])
}

/// How tall one slot is, without laying it out.
fn slot_height(slot: &super::local::Slot, items: &[Item<'_>], cache: &BlockCache) -> usize {
    match slot.kind {
        SlotKind::Next => 0,
        SlotKind::Block(_) => item_of(items, slot.num)
            .and_then(|it| cache.known_height(slot.num, it.block))
            .unwrap_or(1),
        _ => 1,
    }
}

/// One slot's rows this frame. A slot with nothing to show draws nothing.
fn slot_rows(
    slot: &super::local::Slot,
    items: &[Item<'_>],
    cache: &mut BlockCache,
    t: &HistoryTheme,
    width: usize,
) -> Vec<Line<'static>> {
    match &slot.kind {
        SlotKind::Next => Vec::new(),
        SlotKind::Empty => vec![Line::default()],
        SlotKind::Notice { text, level, .. } => notice_block(text, *level, t),
        SlotKind::Block(_) => match item_of(items, slot.num) {
            Some(it) => cache.rows(it, t),
            None => vec![Line::default()],
        },
    }
    .into_iter()
    .flat_map(|line| {
        if crate::ui::tui::text::display_width(&line.to_string()) > width {
            super::render::block::wrap_rows(std::iter::once(line), width)
        } else {
            vec![line]
        }
    })
    .collect()
}
