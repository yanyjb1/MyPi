//! Aiming the window at a block: resolve the target, wait for its content, land.

use super::local::section::Idx;
use super::local::slot::SlotKind;
use super::prefetch::{Aim, Head, Start, walk};
use super::zone::HistoryZone;

/// What a caller asks the window to point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// One block of the finished conversation; an undrawable block looks older.
    Block(i64),
    /// One round's head; an undrawable head looks newer.
    Round(i64),
    /// The running round's head.
    Running,
}

/// The pending focus: resolved again whenever content or structure lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Block { id: i64, newer: bool },
    Round(i64),
    Position { idx: Idx, newer: bool },
    Running,
}

/// One candidate's outcome.
enum Step {
    Land,
    Wait,
    Skip,
}

/// What one fixed slot holds, copied out of the section.
enum Which {
    Next,
    Notice,
    Block(i64),
}

impl HistoryZone {
    /// Point the window at a target. The block's content, and the rows above it, must be
    /// in hand before the window moves.
    pub fn focus(&mut self, target: Focus) {
        self.pending_focus = Some(match target {
            Focus::Block(id) => Target::Block { id, newer: false },
            Focus::Round(turn) => Target::Round(turn),
            Focus::Running => Target::Running,
        });
        self.settle_focus();
    }

    /// Point the window at one render position; `older` is the direction to keep looking.
    pub(super) fn focus_at(&mut self, idx: Idx, older: bool) {
        self.pending_focus = Some(Target::Position { idx, newer: !older });
        self.settle_focus();
    }

    /// The same aim with a block head as the first candidate.
    pub(super) fn focus_block(&mut self, id: i64, newer: bool) {
        self.pending_focus = Some(Target::Block { id, newer });
        self.settle_focus();
    }

    /// Land the pending focus when its content is here; otherwise keep waiting.
    pub(super) fn settle_focus(&mut self) {
        let Some(target) = self.pending_focus else {
            return;
        };
        let done = match target {
            Target::Block { id, newer } => self.settle_block(id, newer),
            Target::Round(turn) => self.settle_round(turn),
            Target::Position { idx, newer } => self.settle_position(idx, newer),
            Target::Running => self.settle_running(),
        };
        if done {
            self.pending_focus = None;
            self.ensure_around();
        }
    }

    fn settle_round(&mut self, turn: i64) -> bool {
        let head = self
            .structure
            .rounds()
            .iter()
            .find(|round| round.id == turn)
            .and_then(|round| round.ranges.first())
            .map(|[lo, _]| *lo);
        match head {
            Some(id) => self.settle_block(id, true),
            None => true,
        }
    }

    fn settle_block(&mut self, id: i64, newer: bool) -> bool {
        if self.local.idx_of(id).is_none() {
            return false;
        }
        let Some(turn) = self.turn_of(id) else {
            return false;
        };
        let ids = self.round_ids(turn);
        let Some(at) = ids.iter().position(|held| *held == id) else {
            return true;
        };
        let order: Vec<i64> = if newer {
            ids[at..].to_vec()
        } else {
            ids[..=at].iter().rev().copied().collect()
        };
        for id in order {
            if let Some(idx) = self.local.idx_of(id) {
                match self.step(idx) {
                    Step::Land => return true,
                    Step::Wait => return false,
                    Step::Skip => {}
                }
            }
        }
        true
    }

    fn settle_position(&mut self, idx: Idx, newer: bool) -> bool {
        match idx {
            Idx::Turn(at) => {
                let len = self.local.running().len() as u32;
                if at >= len {
                    return true;
                }
                let order: Vec<u32> = if newer {
                    (at..len).collect()
                } else {
                    (0..=at).rev().collect()
                };
                for pos in order {
                    match self.step(Idx::Turn(pos)) {
                        Step::Land => return true,
                        Step::Wait => return false,
                        Step::Skip => {}
                    }
                }
                true
            }
            Idx::Fixed(num) => {
                let which = match self.local.slot(idx).map(|slot| &slot.kind) {
                    Some(SlotKind::Next) => Which::Next,
                    Some(SlotKind::Notice { .. }) => Which::Notice,
                    Some(SlotKind::Block(id)) => Which::Block(*id),
                    None => return true,
                };
                match which {
                    Which::Next => match self.block_near(num, false) {
                        Some(id) => self.settle_block(id, true),
                        None => true,
                    },
                    Which::Notice => match self.step(idx) {
                        Step::Land => true,
                        Step::Wait => false,
                        Step::Skip => match self.block_near(num, !newer) {
                            Some(id) => self.settle_block(id, newer),
                            None => true,
                        },
                    },
                    Which::Block(id) => self.settle_block(id, newer),
                }
            }
        }
    }

    fn settle_running(&mut self) -> bool {
        let len = self.local.running().len();
        for pos in 0..len {
            match self.step(Idx::Turn(pos as u32)) {
                Step::Land => return true,
                Step::Wait => return false,
                Step::Skip => {}
            }
        }
        true
    }

    fn step(&mut self, idx: Idx) -> Step {
        let line = self
            .focus_line
            .min(usize::from(self.height.saturating_sub(1)));
        let aim = self.with_draw(|local, products, run, range, opts| {
            range.aim(run, local, products, opts, idx, line)
        });
        match aim {
            Aim::Empty => Step::Skip,
            Aim::Missing(blocks) => {
                if self.ask_blocks(&blocks) {
                    Step::Wait
                } else {
                    Step::Skip
                }
            }
            Aim::Top { top, missing } => {
                if self.ask_blocks(&missing) {
                    return Step::Wait;
                }
                self.with_draw(|_, _, _, range, _| range.pin(top));
                Step::Land
            }
        }
    }

    /// Ask for the rounds that hold these missing blocks; `false` when none is reachable.
    fn ask_blocks(&mut self, blocks: &[i64]) -> bool {
        let mut turns: Vec<i64> = Vec::new();
        for block in blocks {
            let Some(turn) = self.turn_of(*block) else {
                continue;
            };
            if !turns.contains(&turn) {
                turns.push(turn);
            }
        }
        let asked = !turns.is_empty();
        for turn in turns {
            self.ask_turn(turn);
        }
        asked
    }

    fn round_ids(&self, turn: i64) -> Vec<i64> {
        self.structure
            .rounds()
            .iter()
            .find(|round| round.id == turn)
            .map(|round| round.ranges.iter().flat_map(|[lo, hi]| *lo..=*hi).collect())
            .unwrap_or_default()
    }

    /// The running round's first position that draws, if any.
    pub(super) fn running_head(&mut self) -> Option<u32> {
        let len = self.local.running().len();
        for pos in 0..len {
            let idx = Idx::Turn(pos as u32);
            if self.draws(idx) {
                return Some(pos as u32);
            }
        }
        None
    }

    fn draws(&mut self, idx: Idx) -> bool {
        self.with_draw(|local, products, run, _, opts| {
            !matches!(run.head(local, products, opts, idx), Head::Empty)
        })
    }

    /// The nearest block at or past this fixed position, in one direction.
    pub(super) fn block_near(&self, num: i64, older: bool) -> Option<i64> {
        let mut cursor = Some(Idx::Fixed(num));
        while let Some(Idx::Fixed(at)) = cursor {
            if let Some(SlotKind::Block(id)) =
                self.local.slot(Idx::Fixed(at)).map(|slot| &slot.kind)
                && *id > 0
            {
                return Some(*id);
            }
            cursor = if older {
                walk::older(&self.local, Idx::Fixed(at))
            } else {
                walk::newer(&self.local, Idx::Fixed(at))
            };
        }
        None
    }

    /// The position under the sight line; the newest when following.
    pub(super) fn anchor_slot(&mut self) -> Option<Idx> {
        match self.range.start() {
            Start::Latest => {
                let len = self.local.running().len();
                if len > 0 {
                    Some(Idx::Turn((len - 1) as u32))
                } else {
                    self.local.newest().map(Idx::Fixed)
                }
            }
            Start::At(..) => {
                let line = self
                    .focus_line
                    .min(usize::from(self.height.saturating_sub(1)));
                self.with_draw(|local, products, run, range, opts| {
                    range.sight(run, local, products, opts, line)
                })
            }
        }
    }

    /// The finished block at the sight line; the newest block when the sight is running.
    pub(super) fn anchor_block(&mut self) -> Option<i64> {
        match self.anchor_slot()? {
            Idx::Turn(_) => {
                let num = self.local.newest()?;
                self.block_near(num, true)
                    .or_else(|| self.block_near(num, false))
            }
            Idx::Fixed(num) => self
                .block_near(num, true)
                .or_else(|| self.block_near(num, false)),
        }
    }
}
