//! Blocks on demand: one round at a time, and the anchor's neighbours.

use super::super::zone::HistoryZone;

use crate::ui::tui::zone::Request;

impl HistoryZone {
    /// Ask for one round's blocks, unless they are here or already asked for.
    pub(crate) fn ask_turn(&mut self, turn: i64) {
        let ranges: Vec<[i64; 2]> = match self
            .structure
            .rounds()
            .iter()
            .find(|round| round.id == turn)
        {
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
            self.outbox.push(Request::Blocks {
                ranges: vec![[lo, hi]],
            });
        }
    }

    /// The round a block belongs to.
    pub(crate) fn turn_of(&self, block: i64) -> Option<i64> {
        self.structure
            .rounds()
            .iter()
            .find(|round| round.owns(block))
            .map(|round| round.id)
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
            let rounds = self.structure.rounds();
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
