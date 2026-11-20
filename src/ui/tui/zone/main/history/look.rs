//! Where the reader stands: one slot, one round, or the first block of a round.

use super::focus::Focus;
use super::local::section::Idx;
use super::local::slot::SlotKind;
use super::prefetch::{Start, walk};
use super::zone::HistoryZone;

impl HistoryZone {
    /// One whole slot older or newer: its head under the sight line.
    pub(super) fn step_slot(&mut self, older: bool) {
        let Some(from) = self.anchor_slot() else {
            return;
        };
        let mut cursor = if older {
            walk::older(&self.local, from)
        } else {
            walk::newer(&self.local, from)
        };
        while let Some(idx) = cursor {
            if self.local.holds_block(idx) {
                self.focus_at(idx, older);
                return;
            }
            cursor = if older {
                walk::older(&self.local, idx)
            } else {
                walk::newer(&self.local, idx)
            };
        }
        if !older {
            self.range.follow();
        }
    }

    /// Point at the newest finished round's head.
    fn focus_newest_round(&mut self) {
        let newest = self.structure.turns().next().map(|round| round.id);
        if let Some(turn) = newest {
            self.focus(Focus::Round(turn));
        }
    }

    /// One whole round older or newer: its head under the sight line.
    pub(super) fn turn_step(&mut self, older: bool) {
        if matches!(self.range.start(), Start::Latest) {
            if !older {
                self.range.follow();
                return;
            }
            if self.running_head().is_some() {
                self.focus(Focus::Running);
            } else {
                self.focus_newest_round();
            }
            return;
        }
        let Some(slot) = self.anchor_slot() else {
            return;
        };
        if let Idx::Turn(at) = slot {
            if !older {
                self.range.follow();
                return;
            }
            match self.running_head() {
                Some(head) if head != at => self.focus(Focus::Running),
                _ => self.focus_newest_round(),
            }
            return;
        }
        let Idx::Fixed(num) = slot else {
            return;
        };
        let direct = matches!(
            self.local.slot(slot).map(|slot| &slot.kind),
            Some(SlotKind::Block { .. })
        );
        let Some(block) = self.block_near(num, older) else {
            return;
        };
        let Some(turn) = self.turn_of(block) else {
            return;
        };
        let next = {
            let rounds: Vec<_> = self.structure.turns().collect();
            let Some(at) = rounds.iter().position(|round| round.id == turn) else {
                return;
            };
            if direct {
                if older {
                    rounds.get(at + 1).map(|round| round.id)
                } else {
                    at.checked_sub(1)
                        .and_then(|index| rounds.get(index))
                        .map(|round| round.id)
                }
            } else {
                Some(turn)
            }
        };
        match next {
            Some(turn) => self.focus(Focus::Round(turn)),
            None if !older => self.range.follow(),
            None => {}
        }
    }

    /// Put the sight line on the first or last block of the anchor's round.
    pub(super) fn align(&mut self, last: bool) {
        let Some(slot) = self.anchor_slot() else {
            return;
        };
        let Idx::Fixed(num) = slot else {
            return;
        };
        let Some(block) = self.block_near(num, true) else {
            return;
        };
        let Some(turn) = self.turn_of(block) else {
            return;
        };
        if !last {
            self.focus(Focus::Round(turn));
            return;
        }
        let id = {
            let rounds: Vec<_> = self.structure.turns().collect();
            let Some(round) = rounds.iter().find(|round| round.id == turn) else {
                return;
            };
            round.ranges.iter().flat_map(|[lo, hi]| *lo..=*hi).max()
        };
        if let Some(id) = id {
            self.focus_block(id, false);
        }
    }

    /// The oldest round's head.
    pub(super) fn first(&mut self) {
        if let Some(turn) = self.structure.turns().last().map(|round| round.id) {
            self.focus(Focus::Round(turn));
        }
    }

    /// One scroll by rows: the screen moves, and the direction it moved stays stocked.
    pub(super) fn scroll(&mut self, older: bool, rows: usize) {
        let height = usize::from(self.height).max(1);
        let missing = self.with_draw(|local, run, range, opts| {
            let missing = range.scroll(run, local, opts, height, older, rows);
            // The run's reach moved with the screen; the walk reads it now, not next draw.
            let reach = run.reach();
            local.set_reach(reach);
            missing
        });
        self.look_ahead(older, missing);
    }
}
