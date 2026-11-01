//! The structure snapshot: the rounds of the attached session and the block ids each owns.

use std::ops::RangeInclusive;

use super::event::Cut;

/// One round: its id, and the closed intervals of block ids it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub id: i64,
    pub blocks: Vec<RangeInclusive<i64>>,
}

impl Turn {
    pub fn new(id: i64, ranges: Vec<[i64; 2]>) -> Turn {
        Turn {
            id,
            blocks: ranges.into_iter().map(|[lo, hi]| lo..=hi).collect(),
        }
    }
}

/// The kept rounds, newest first.
#[derive(Debug, Clone, Default)]
pub struct Structure {
    turns: Vec<Turn>,
}

impl Structure {
    /// Record one round: the same round replaces its old row.
    pub fn set(&mut self, turn: Turn) {
        self.turns.retain(|held| held.id != turn.id);
        self.turns.insert(0, turn);
    }

    /// The round's intervals.
    pub fn ranges(&self, turn: i64) -> Option<&[RangeInclusive<i64>]> {
        self.turns
            .iter()
            .find(|held| held.id == turn)
            .map(|held| held.blocks.as_slice())
    }

    /// The interval that owns this block.
    pub fn interval(&self, block: i64) -> Option<(i64, i64)> {
        self.turns
            .iter()
            .flat_map(|held| held.blocks.iter())
            .find(|range| range.contains(&block))
            .map(|range| (*range.start(), *range.end()))
    }

    /// The round that owns this block.
    pub fn turn_of(&self, block: i64) -> Option<i64> {
        self.turns
            .iter()
            .find(|held| held.blocks.iter().any(|range| range.contains(&block)))
            .map(|held| held.id)
    }

    /// Replace the whole structure; `turns` is newest first.
    pub fn replace(&mut self, turns: Vec<Turn>) {
        self.turns = turns;
    }

    /// Cut the list back to one block, the same way the warehouse did.
    pub fn rewind(&mut self, turn: i64, block: i64, cut: Cut) {
        let Some(at) = self.turns.iter().position(|held| held.id == turn) else {
            return;
        };
        match cut {
            Cut::Retyped { .. } => {
                self.turns.drain(..=at);
            }
            Cut::Boundary { .. } => {
                self.turns.drain(..at);
            }
            Cut::Rescued { turn: rescued } => {
                self.turns.drain(..at);
                self.turns[0].id = rescued;
                self.turns[0].blocks = head(&self.turns[0].blocks, block);
            }
        }
    }

    /// Newest first.
    pub fn iter(&self) -> impl Iterator<Item = &Turn> {
        self.turns.iter()
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }
}

/// The kept head of a run of intervals, up to and including `upto`.
fn head(blocks: &[RangeInclusive<i64>], upto: i64) -> Vec<RangeInclusive<i64>> {
    let mut out = Vec::new();
    for range in blocks {
        if *range.start() > upto {
            break;
        }
        out.push(*range.start()..=(*range.end()).min(upto));
    }
    out
}
