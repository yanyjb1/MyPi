//! The structure cache: the rounds of the attached session, and the block id intervals each owns.

use std::ops::RangeInclusive;

use crate::ipc::contract::Block;

/// One round: its id, and the intervals of block ids it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub id: i64,
    pub blocks: Vec<RangeInclusive<i64>>,
}

/// The kept rounds, newest first.
#[derive(Debug, Clone, Default)]
pub struct Structure {
    turns: Vec<Turn>,
}

impl Structure {
    pub fn new(turns: Vec<Turn>) -> Structure {
        Structure { turns }
    }

    /// Record one round: the same round replaces its old row.
    pub fn set(&mut self, turn: Turn) {
        self.turns.retain(|held| held.id != turn.id);
        self.turns.insert(0, turn);
    }

    /// Append one landed batch to its round, opening an interval where ids are not contiguous.
    pub fn add_blocks(&mut self, turn: i64, blocks: &[Block]) {
        if blocks.is_empty() {
            return;
        }
        if !self.turns.iter().any(|held| held.id == turn) {
            self.turns.insert(
                0,
                Turn {
                    id: turn,
                    blocks: Vec::new(),
                },
            );
        }
        let Some(held) = self.turns.iter_mut().find(|held| held.id == turn) else {
            return;
        };
        let mut ids: Vec<i64> = blocks.iter().map(|block| block.id).collect();
        ids.sort_unstable();
        for id in ids {
            extend(&mut held.blocks, id);
        }
    }

    /// The round's intervals, in the order they were opened.
    pub fn ranges(&self, turn: i64) -> Option<&[RangeInclusive<i64>]> {
        self.turns
            .iter()
            .find(|held| held.id == turn)
            .map(|held| held.blocks.as_slice())
    }

    pub fn first_id(&self) -> Option<i64> {
        self.turns
            .iter()
            .filter_map(|held| held.blocks.first())
            .map(|range| *range.start())
            .min()
    }

    pub fn last_id(&self) -> Option<i64> {
        self.turns
            .iter()
            .filter_map(|held| held.blocks.last())
            .map(|range| *range.end())
            .max()
    }

    /// Replace the whole structure; `turns` is newest first.
    pub fn replace(&mut self, structure: Structure) {
        self.turns = structure.turns;
    }

    /// Newest first.
    pub fn iter(&self) -> impl Iterator<Item = &Turn> {
        self.turns.iter()
    }

    /// The round that owns this block.
    pub fn turn_of(&self, block: i64) -> Option<i64> {
        self.turns
            .iter()
            .find(|held| held.blocks.iter().any(|range| range.contains(&block)))
            .map(|held| held.id)
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }
}

/// Put one id into the interval list: extend the last interval, ignore a held id, else open one.
fn extend(blocks: &mut Vec<RangeInclusive<i64>>, id: i64) {
    if blocks.iter().any(|range| range.contains(&id)) {
        return;
    }
    if let Some(last) = blocks.last_mut()
        && *last.end() + 1 == id
    {
        *last = *last.start()..=id;
        return;
    }
    blocks.push(id..=id);
}
