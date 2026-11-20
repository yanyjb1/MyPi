//! Blocks on demand: one round at a time, and the anchor's neighbours.

use super::super::zone::HistoryZone;

impl HistoryZone {
    /// Stock the direction the reader just moved: the blocks past the screen's edge that hold no
    /// content go out as one ask, together with what the move itself landed on. A direction that
    /// is already stocked asks nothing.
    pub(crate) fn look_ahead(&mut self, older: bool, landed: Vec<i64>) {
        let mut gaps = self.local.dry(older, self.overscan);
        for block in &landed {
            if !gaps.contains(block) {
                gaps.push(*block);
            }
        }
        let mut ranges: Vec<[i64; 2]> = Vec::new();
        for block in gaps.iter() {
            let wanted = match self.turn_of(*block) {
                Some(turn) => self.round_ranges(turn),
                None => vec![[*block, *block]],
            };
            for range in wanted {
                if !ranges.contains(&range) {
                    ranges.push(range);
                }
            }
        }
        ranges.retain(|range| {
            !self.asking.contains(range) && !(range[0]..=range[1]).all(|id| self.local.has(id))
        });
        if ranges.is_empty() {
            return;
        }
        for range in &ranges {
            self.asking.insert(*range);
        }
        self.want_blocks(&ranges);
    }

    /// Ask for one round's blocks, unless they are here or already asked for.
    pub(crate) fn ask_turn(&mut self, turn: i64) {
        let ranges: Vec<[i64; 2]> = match self.structure.turns().find(|round| round.id == turn) {
            Some(round) => round.ranges.clone(),
            None => return,
        };
        for [lo, hi] in ranges {
            if self.asking.contains(&[lo, hi]) {
                continue;
            }
            if (lo..=hi).all(|id| self.local.has(id)) {
                continue;
            }
            self.asking.insert([lo, hi]);
            self.want_blocks(&[[lo, hi]]);
        }
    }

    /// The round a block belongs to.
    pub(crate) fn turn_of(&self, block: i64) -> Option<i64> {
        self.structure
            .turns()
            .find(|round| round.owns(block))
            .map(|round| round.id)
    }

    /// One round's block ranges, as the structure names them.
    fn round_ranges(&self, turn: i64) -> Vec<[i64; 2]> {
        self.structure
            .turns()
            .find(|round| round.id == turn)
            .map(|round| round.ranges.clone())
            .unwrap_or_default()
    }

    /// Keep the anchor's own round and its neighbours in hand.
    pub(crate) fn ensure_around(&mut self) {
        let Some(block) = self.anchor_block() else {
            return;
        };
        let Some(turn) = self.turn_of(block) else {
            return;
        };
        let wanted: Vec<i64> = {
            let rounds: Vec<_> = self.structure.turns().collect();
            let Some(at) = rounds.iter().position(|round| round.id == turn) else {
                return;
            };
            [-1i64, 0, 1]
                .iter()
                .filter_map(|offset| {
                    let index = at as i64 + offset;
                    (index >= 0 && (index as usize) < rounds.len())
                        .then(|| rounds[index as usize].id)
                })
                .collect()
        };
        for turn in wanted {
            self.ask_turn(turn);
        }
    }
}
