//! The viewport: the sight line, the seam, and the rows the window maps to.

use super::line::Row;
use super::local::{Local, SlotKind};

/// A spot inside one slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
    /// Cards: a row index.
    Row(usize),
    /// Prose: a natural line and the cells before it.
    Natural { line: usize, off: usize },
}

/// A slot number and a spot inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub num: i64,
    pub spot: Spot,
}

/// Where the seam sits: on a spot, or at the end of the sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seam {
    At(Anchor),
    End,
}

/// Which way the reader moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Older,
    Newer,
}

pub struct Viewport {
    line: u16,
    seam: Seam,
    rows: u16,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            line: 3,
            seam: Seam::End,
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

    /// The slot the seam is on; the newest slot when the seam is at the end.
    pub fn anchor(&self, local: &Local) -> i64 {
        match self.seam {
            Seam::At(anchor) => anchor.num,
            Seam::End => local.newest().unwrap_or(0),
        }
    }

    /// Put a slot's head right under the sight line.
    pub fn focus_num(&mut self, num: i64) {
        self.seam = Seam::At(Anchor {
            num,
            spot: Spot::Row(0),
        });
    }

    /// One slot older or newer. A dead notice is stepped over.
    pub fn step(&mut self, local: &Local, dir: Dir) {
        let start = match self.seam {
            Seam::End => {
                let Some(newest) = local.newest() else {
                    return;
                };
                if dir == Dir::Newer {
                    return;
                }
                newest
            }
            Seam::At(anchor) => match dir {
                Dir::Older => anchor.num,
                Dir::Newer => anchor.num + 1,
            },
        };
        let mut num = match self.seam {
            Seam::End => start,
            Seam::At(_) => match dir {
                Dir::Older => start - 1,
                Dir::Newer => start,
            },
        };
        loop {
            match local.at(num) {
                None => {
                    if dir == Dir::Newer && num > start.saturating_sub(1) {
                        self.seam = Seam::End;
                    }
                    return;
                }
                Some(slot) if slot.kind == SlotKind::Next => {
                    num = match dir {
                        Dir::Older => num - 1,
                        Dir::Newer => num + 1,
                    };
                }
                Some(_) => {
                    self.seam = Seam::At(Anchor {
                        num,
                        spot: Spot::Row(0),
                    });
                    return;
                }
            }
        }
    }

    /// Follow the newest block again.
    pub fn follow(&mut self) {
        self.seam = Seam::End;
    }

    /// The oldest slot under the sight line.
    pub fn first(&mut self, local: &Local) {
        if let Some(num) = local.oldest() {
            self.seam = Seam::At(Anchor {
                num,
                spot: Spot::Row(0),
            });
        }
    }

    /// A screen older or newer: whole slots until the rows are covered.
    pub fn page(&mut self, local: &Local, height_of: impl Fn(i64) -> usize, dir: Dir) {
        let budget = usize::from(self.rows().max(1));
        if matches!(self.seam, Seam::End) {
            if dir == Dir::Newer {
                return;
            }
            let Some(newest) = local.newest() else {
                return;
            };
            self.seam = Seam::At(Anchor {
                num: newest,
                spot: Spot::Row(0),
            });
        }
        let mut covered = 0usize;
        let mut num = match self.seam {
            Seam::At(anchor) => anchor.num,
            Seam::End => return,
        };
        let step = match dir {
            Dir::Older => -1,
            Dir::Newer => 1,
        };
        let mut last = num;
        while covered < budget {
            num += step;
            match local.at(num) {
                None => break,
                Some(slot) if slot.kind == SlotKind::Next => continue,
                Some(_) => {}
            }
            covered += height_of(num) + 1;
            last = num;
        }
        match local.at(last) {
            None => {
                if dir == Dir::Newer {
                    self.seam = Seam::End;
                }
            }
            Some(_) => {
                self.seam = Seam::At(Anchor {
                    num: last,
                    spot: Spot::Row(0),
                })
            }
        }
    }

    /// The seam as it stands.
    pub fn seam(&self) -> Seam {
        self.seam
    }

    /// Put the seam somewhere; the caller resolves the spot.
    pub fn set_seam(&mut self, seam: Seam) {
        self.seam = seam;
    }

    /// Lay this frame's rows out: above the seam, below it, then the pinned rows.
    pub fn compose(
        &self,
        local: &Local,
        mut rows_of: impl FnMut(i64) -> Vec<Row>,
        seam_mid: usize,
        tail: &[Row],
        pinned: &[Row],
    ) -> Vec<Row> {
        let body = usize::from(self.rows).saturating_sub(pinned.len());
        if body == 0 {
            return pinned.to_vec();
        }
        let (start, mid) = match self.seam {
            Seam::End => (local.newest().map(|n| n + 1).unwrap_or(0), 0usize),
            Seam::At(anchor) => (anchor.num, seam_mid),
        };
        let end = matches!(self.seam, Seam::End);
        let above_budget = if end {
            body
        } else {
            usize::from(self.line).min(body)
        };

        let live = local.at(start).filter(|slot| slot.kind != SlotKind::Next);
        let seam_rows = live.map(|_| rows_of(start)).unwrap_or_default();
        let seam_has_rows = !seam_rows.is_empty();
        let mut chunks: Vec<Vec<Row>> = Vec::new();
        let mut len = 0usize;
        if end && !tail.is_empty() {
            len += tail.len() + 1;
            chunks.push(tail.to_vec());
        }
        let mut num = start - 1;
        while len < above_budget {
            let Some(slot) = local.at(num) else { break };
            if slot.kind == SlotKind::Next {
                num -= 1;
                continue;
            }
            let rows = rows_of(num);
            if !rows.is_empty() {
                len += rows.len() + 1;
                chunks.push(rows);
            }
            num -= 1;
        }
        let mut above: Vec<Row> = Vec::new();
        for (index, chunk) in chunks.into_iter().rev().enumerate() {
            if index > 0 {
                above.push(Row::blank());
            }
            above.extend(chunk);
        }
        if !above.is_empty() && seam_has_rows {
            above.push(Row::blank());
        }
        if mid > 0 {
            above.extend(seam_rows.iter().take(mid).cloned());
        }
        let overflow = above.len().saturating_sub(above_budget);
        let above: Vec<Row> = above.into_iter().skip(overflow).collect();

        let mut below: Vec<Row> = Vec::new();
        let mut below_len = 0usize;
        let below_budget = body.saturating_sub(above.len());
        if seam_has_rows {
            let rest: Vec<Row> = seam_rows.iter().skip(mid).cloned().collect();
            below_len += rest.len();
            below.extend(rest);
        }
        let mut num = start + 1;
        while below_len < below_budget {
            match local.at(num) {
                None => break,
                Some(slot) if slot.kind == SlotKind::Next => {
                    num += 1;
                    continue;
                }
                Some(_) => {}
            }
            let rows = rows_of(num);
            if !rows.is_empty() {
                below_len += rows.len() + 1;
                below.push(Row::blank());
                below.extend(rows);
            }
            num += 1;
        }
        if local.at(num).is_none() && below_len < below_budget && !tail.is_empty() {
            if !below.is_empty() || !above.is_empty() {
                below.push(Row::blank());
            }
            below.extend(tail.iter().cloned());
        }
        let below: Vec<Row> = below.into_iter().take(below_budget).collect();

        let mut out: Vec<Row> = above;
        out.extend(below);
        out.truncate(body);
        while out.len() < body {
            out.push(Row::blank());
        }
        out.extend(pinned.iter().cloned());
        out
    }
}
