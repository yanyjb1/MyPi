//! The viewport: the sight line, the walk out from it, and the rows that walk collects.

use ratatui::text::Line;

use super::local::{Local, SlotKind};

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

/// The visible region: a seam, and the rows the main zone handed down.
pub struct Viewport {
    /// The sight line, counted from the top of the zone.
    line: u16,
    anchor: Anchor,
    /// The newest block sits against the pinned rows.
    bottom: bool,
    /// Rows the main zone handed down.
    rows: u16,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            line: 3,
            anchor: Anchor { num: 0, row: 0 },
            bottom: true,
            rows: 0,
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

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn anchor(&self) -> i64 {
        self.anchor.num
    }

    /// Put a slot right under the sight line.
    pub fn focus_num(&mut self, num: i64) {
        self.anchor = Anchor { num, row: 0 };
        self.bottom = false;
    }

    /// One slot older or newer. The bottom's seam is one past the newest slot, so one
    /// step older lands on the newest; one step newer stays at the bottom.
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
        }
    }

    /// A screen older or newer: whole slots until the rows are covered.
    pub fn page(&mut self, local: &Local, height_of: impl Fn(i64) -> usize, dir: Dir) {
        let budget = usize::from(self.rows().max(1));
        let step = match dir {
            Dir::Older => -1,
            Dir::Newer => 1,
        };
        let mut covered = 0usize;
        let mut num = self.anchor.num;
        while covered < budget {
            num += step;
            if local.at(num).is_none() {
                break;
            }
            covered += height_of(num) + 1;
        }
        if local.at(num).is_some() {
            self.anchor = Anchor { num, row: 0 };
            self.bottom = false;
        }
    }

    /// A resize keeps the seam: the same slot, a row that still exists inside it.
    pub fn on_resize(&mut self, local: &Local, height_of: impl Fn(i64) -> usize) {
        if self.bottom {
            return;
        }
        if local.at(self.anchor.num).is_none() {
            self.anchor = Anchor {
                num: local.newest().unwrap_or(0),
                row: 0,
            };
            return;
        }
        let height = height_of(self.anchor.num).max(1);
        self.anchor.row = self.anchor.row.min(height - 1);
    }

    /// Lay this frame's rows out: above the seam, below it, then the pinned rows.
    ///
    /// A dead notice draws nothing and does not break a separator: it is stepped over, not
    /// walked through.
    pub fn compose(
        &mut self,
        local: &Local,
        mut rows_of: impl FnMut(i64) -> Vec<Line<'static>>,
        tail: &[Line<'static>],
        pinned: &[Line<'static>],
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
        if self.bottom {
            self.anchor = Anchor {
                num: local.newest().map(|n| n + 1).unwrap_or(0),
                row: 0,
            };
        }
        let seam = self.anchor;

        let mut out: Vec<Line<'static>> = Vec::with_capacity(body);
        let mut head: Vec<Line<'static>> = Vec::new();
        let mut visible = false;

        // The seam's own leading rows.
        if let Some(slot) = local.at(seam.num)
            && slot.kind != SlotKind::Next
        {
            let rows = rows_of(slot.num);
            visible = !rows.is_empty();
            head.extend(rows.into_iter().take(seam.row));
        }

        // Older, until the sight line is covered.
        let mut num = seam.num - 1;
        while head.len() < above {
            let Some(slot) = local.at(num) else { break };
            if slot.kind == SlotKind::Next {
                num -= 1;
                continue;
            }
            let rows = rows_of(slot.num);
            if !rows.is_empty() && visible {
                head.push(Line::default());
            }
            visible |= !rows.is_empty();
            head.extend(rows);
            num -= 1;
        }
        let drop = head.len().saturating_sub(above);
        out.extend(head.into_iter().skip(drop));
        out.truncate(above);

        // Below the seam: the seam's remaining rows, then newer slots.
        let mut drew = !out.is_empty();
        if let Some(slot) = local.at(seam.num)
            && slot.kind != SlotKind::Next
        {
            let rows = rows_of(slot.num);
            let rest: Vec<Line<'static>> = rows.into_iter().skip(seam.row).collect();
            if !rest.is_empty() && drew {
                out.push(Line::default());
            }
            drew |= !rest.is_empty();
            out.extend(rest);
        }
        let mut num = seam.num + 1;
        while out.len() < body {
            let Some(slot) = local.at(num) else { break };
            if slot.kind == SlotKind::Next {
                num += 1;
                continue;
            }
            let rows = rows_of(slot.num);
            if !rows.is_empty() && drew {
                out.push(Line::default());
            }
            drew |= !rows.is_empty();
            out.extend(rows);
            num += 1;
        }

        // Past the last slot: what a running turn has said so far.
        if local.at(num).is_none() && !tail.is_empty() {
            if drew {
                out.push(Line::default());
            }
            out.extend(tail.iter().cloned());
        }

        // Short of rows: the conversation ended. The pinned rows keep their place.
        out.truncate(body);
        while out.len() < body {
            out.push(Line::default());
        }
        out.extend(pinned.iter().cloned());
        out
    }
}
