// The structural window: which blocks each round holds.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub id: i64,
    pub ranges: Vec<[i64; 2]>,
    pub compact: bool,
}

impl Turn {
    pub fn new(id: i64, ranges: Vec<[i64; 2]>, compact: bool) -> Turn {
        Turn {
            id,
            ranges,
            compact,
        }
    }

    pub fn owns(&self, block: i64) -> bool {
        self.ranges
            .iter()
            .any(|[lo, hi]| *lo <= block && block <= *hi)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Structure {
    // Keyed by round id: key order is round order.
    turns: BTreeMap<i64, Turn>,
}

impl Structure {
    // A named round arrives; an unnamed single block cuts to that block, everything after it going.
    pub fn update(&mut self, turn: Option<i64>, blocks: &[[i64; 2]], compact: bool) {
        let Some(id) = turn else {
            if let [block] = blocks
                && block[0] == block[1]
            {
                self.cut_to(block[0]);
            }
            return;
        };
        self.turns
            .insert(id, Turn::new(id, blocks.to_vec(), compact));
    }

    pub fn owns(&self, block: i64) -> bool {
        self.turns.values().any(|turn| turn.owns(block))
    }

    // Newest first.
    pub fn turns(&self) -> impl DoubleEndedIterator<Item = &Turn> {
        self.turns.values().rev()
    }

    fn cut_to(&mut self, block: i64) {
        let Some((id, compact, kept)) = self
            .turns
            .iter()
            .find(|(_, turn)| turn.owns(block))
            .map(|(id, turn)| (*id, turn.compact, head(&turn.ranges, block)))
        else {
            return;
        };
        self.turns.split_off(&id);
        if !kept.is_empty() {
            self.turns.insert(id, Turn::new(id, kept, compact));
        }
    }
}

fn head(ranges: &[[i64; 2]], upto: i64) -> Vec<[i64; 2]> {
    let mut out = Vec::new();
    for [lo, hi] in ranges {
        if *lo > upto {
            break;
        }
        out.push([*lo, (*hi).min(upto)]);
    }
    out
}
