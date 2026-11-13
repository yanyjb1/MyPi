//! The structural window a zone reads: the rounds and the block ranges each owns.
//!
//! The app builds it from the session's snapshot; zones never name a session type.

/// One round: its id, the closed block ranges it owns, and whether it is a compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    pub id: i64,
    pub ranges: Vec<[i64; 2]>,
    pub compact: bool,
}

impl Round {
    pub fn new(id: i64, ranges: Vec<[i64; 2]>, compact: bool) -> Round {
        Round {
            id,
            ranges,
            compact,
        }
    }

    /// Whether one block id falls in this round.
    pub fn owns(&self, block: i64) -> bool {
        self.ranges
            .iter()
            .any(|[lo, hi]| *lo <= block && block <= *hi)
    }
}

/// Every round the session holds, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Turns {
    rounds: Vec<Round>,
}

impl Turns {
    pub fn of(rounds: Vec<Round>) -> Turns {
        Turns { rounds }
    }

    /// Newest first.
    pub fn rounds(&self) -> &[Round] {
        &self.rounds
    }

    /// Whether any round owns this block.
    pub fn owns(&self, block: i64) -> bool {
        self.rounds.iter().any(|round| round.owns(block))
    }

    /// Every block id the compaction rounds hold.
    pub fn compact_blocks(&self) -> Vec<i64> {
        self.rounds
            .iter()
            .filter(|round| round.compact)
            .flat_map(|round| round.ranges.iter())
            .flat_map(|[lo, hi]| *lo..=*hi)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.rounds.is_empty()
    }

    pub fn clear(&mut self) {
        self.rounds.clear();
    }
}
