//! The prefetch layer: the rows around the sight line, laid one whole piece at a time.

use std::collections::VecDeque;

use super::layout::line::{Ink, Lead, Row};
use super::layout::wrap;
use super::paint::{self, Opts};
use super::product::{Piece, card_rows, pieces};
use super::wire::decode::Kind as BlockKind;
use super::zone::HistoryZone;

use crate::ui::tui::theme::fixed;

/// Where one laid row came from: a local number and the piece inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Origin {
    pub num: i64,
    pub piece: u32,
}

/// One row that is already laid, and where it came from.
#[derive(Debug, Clone)]
pub(crate) struct Laid {
    pub row: Row,
    pub at: Origin,
    /// The blank row that separates this piece from the one before it.
    pub sep: bool,
}

/// The run of laid rows, and the width it was laid at.
#[derive(Default)]
pub(crate) struct Prefetch {
    pub(super) rows: VecDeque<Laid>,
    pub(super) width: usize,
}

impl HistoryZone {
    /// The number the growing tail answers to: right after the newest slot.
    pub(super) fn tail_num(&self) -> i64 {
        self.local.newest().map(|num| num + 1).unwrap_or(0)
    }

    /// How many pieces a slot holds. No row is laid to answer this.
    pub(super) fn piece_count(&mut self, num: i64) -> usize {
        if num == self.tail_num() {
            return usize::from(!self.live.is_empty());
        }
        let Some(slot) = self.local.at(num).cloned() else {
            return 0;
        };
        let id = match slot.kind {
            super::local::SlotKind::Next => return 0,
            super::local::SlotKind::Notice { .. } => return 1,
            super::local::SlotKind::Block(id) => id,
        };
        let Some(kind) = self.blocks.get(&id).map(|block| block.kind) else {
            return 1;
        };
        match kind {
            BlockKind::ToolResult => 0,
            BlockKind::Assistant | BlockKind::Reasoning => {
                let placeholder = self.placeholder.clone();
                let cwd = self.cwd.clone();
                let opts = Opts {
                    thinking: self.thinking,
                    tool: self.tool,
                    placeholder: &placeholder,
                    cwd: &cwd,
                    fold: self.fold,
                };
                let HistoryZone {
                    blocks, products, ..
                } = self;
                let Some(block) = blocks.get(&id) else {
                    return 0;
                };
                products.text(id, || pieces(block, &opts)).len()
            }
            _ => 1,
        }
    }

    /// The piece just after this one, walking towards the newest content.
    pub(super) fn step_newer(&mut self, at: Origin) -> Option<Origin> {
        let count = self.piece_count(at.num);
        if at.piece as usize + 1 < count {
            return Some(Origin {
                num: at.num,
                piece: at.piece + 1,
            });
        }
        let tail = self.tail_num();
        let mut num = at.num + 1;
        while num <= tail {
            if self.piece_count(num) > 0 {
                return Some(Origin { num, piece: 0 });
            }
            num += 1;
        }
        None
    }

    /// The piece just before this one, walking towards the oldest content.
    pub(super) fn step_older(&mut self, at: Origin) -> Option<Origin> {
        if at.piece > 0 {
            return Some(Origin {
                num: at.num,
                piece: at.piece - 1,
            });
        }
        let Some(oldest) = self.local.oldest() else {
            return None;
        };
        let mut num = at.num - 1;
        while num >= oldest {
            let count = self.piece_count(num);
            if count > 0 {
                return Some(Origin {
                    num,
                    piece: (count - 1) as u32,
                });
            }
            num -= 1;
        }
        None
    }

    /// One piece as rows at the width this layer keeps.
    pub(super) fn layer_piece(&mut self, at: Origin) -> Vec<Row> {
        let width = self.prefetch_width();
        if at.num == self.tail_num() {
            let placeholder = self.placeholder.clone();
            let cwd = self.cwd.clone();
            let opts = Opts {
                thinking: self.thinking,
                tool: self.tool,
                placeholder: &placeholder,
                cwd: &cwd,
                fold: self.fold,
            };
            return self.live.rows(&opts, width);
        }
        let Some(slot) = self.local.at(at.num).cloned() else {
            return Vec::new();
        };
        match slot.kind {
            super::local::SlotKind::Next => Vec::new(),
            super::local::SlotKind::Notice { text, level, .. } => {
                wrap::rows(paint::notice::block(&text, level), width)
            }
            super::local::SlotKind::Block(id) => self.block_piece(id, at.piece as usize, width),
        }
    }

    fn block_piece(&mut self, id: i64, piece: usize, width: usize) -> Vec<Row> {
        let placeholder = self.placeholder.clone();
        let cwd = self.cwd.clone();
        let opts = Opts {
            thinking: self.thinking,
            tool: self.tool,
            placeholder: &placeholder,
            cwd: &cwd,
            fold: self.fold,
        };
        let HistoryZone {
            blocks, products, ..
        } = self;
        let Some(block) = blocks.get(&id) else {
            return vec![Row::text(Lead::None, "…", Some(Ink::Fixed(fixed::SYSTEM)))];
        };
        match block.kind {
            BlockKind::ToolResult => Vec::new(),
            BlockKind::ToolRequest => {
                let paired = blocks.get(&(id + 1)).filter(|next| block.answers(next));
                products
                    .fixed(id, || card_rows(block, paired, &opts, width))
                    .to_vec()
            }
            BlockKind::User | BlockKind::Wake => products
                .fixed(id, || card_rows(block, None, &opts, width))
                .to_vec(),
            BlockKind::Assistant | BlockKind::Reasoning => {
                let laid = products.text(id, || pieces(block, &opts));
                match laid.get(piece) {
                    Some(Piece::Natural(lines)) => wrap::rows(lines.clone(), width),
                    Some(Piece::Table(seg)) => paint::table::rows(seg, width),
                    None => Vec::new(),
                }
            }
        }
    }

    /// One more piece on the newer end.
    pub(super) fn extend_newer(&mut self) -> bool {
        let Some(last) = self.prep.rows.back().map(|laid| laid.at) else {
            return self.start_newest();
        };
        if last.num == self.tail_num() {
            return self.regrow(last);
        }
        let mut from = last;
        while let Some(next) = self.step_newer(from) {
            let rows = self.layer_piece(next);
            if rows.is_empty() {
                from = next;
                continue;
            }
            self.prep.rows.push_back(Laid {
                row: Row::blank(),
                at: next,
                sep: true,
            });
            for row in rows {
                self.prep.rows.push_back(Laid {
                    row,
                    at: next,
                    sep: false,
                });
            }
            return true;
        }
        false
    }

    /// The growing tail changed: its rows are replaced where they stand.
    fn regrow(&mut self, at: Origin) -> bool {
        let laid: Vec<Row> = self
            .prep
            .rows
            .iter()
            .filter(|item| item.at == at && !item.sep)
            .map(|item| item.row.clone())
            .collect();
        let rows = self.layer_piece(at);
        if laid == rows {
            return false;
        }
        while self.prep.rows.back().is_some_and(|item| item.at == at) {
            self.prep.rows.pop_back();
        }
        if !self.prep.rows.is_empty() {
            self.prep.rows.push_back(Laid {
                row: Row::blank(),
                at,
                sep: true,
            });
        }
        for row in rows {
            self.prep.rows.push_back(Laid {
                row,
                at,
                sep: false,
            });
        }
        true
    }

    /// One more piece on the older end.
    pub(super) fn extend_older(&mut self) -> bool {
        let Some(first) = self.prep.rows.front().map(|laid| laid.at) else {
            return self.start_newest();
        };
        let Some(prev) = self.step_older(first) else {
            return false;
        };
        let rows = self.layer_piece(prev);
        for row in rows.into_iter().rev() {
            self.prep.rows.push_front(Laid {
                row,
                at: prev,
                sep: false,
            });
        }
        self.prep.rows.push_front(Laid {
            row: Row::blank(),
            at: prev,
            sep: true,
        });
        true
    }

    /// Nothing is laid yet: begin at the newest piece there is.
    fn start_newest(&mut self) -> bool {
        let mut num = self.tail_num();
        loop {
            if self.piece_count(num) > 0 {
                self.lay_from(Origin { num, piece: 0 });
                return !self.prep.rows.is_empty();
            }
            let Some(oldest) = self.local.oldest() else {
                return false;
            };
            if num <= oldest {
                return false;
            }
            num -= 1;
        }
    }

    /// Lay this one piece; walking continues from here in either direction.
    pub(super) fn lay_from(&mut self, at: Origin) {
        self.prep.rows.clear();
        let mut origin = at;
        loop {
            let rows = self.layer_piece(origin);
            if !rows.is_empty() {
                for row in rows {
                    self.prep.rows.push_back(Laid {
                        row,
                        at: origin,
                        sep: false,
                    });
                }
                return;
            }
            match self.step_newer(origin) {
                Some(next) => origin = next,
                None => return,
            }
        }
    }

    /// The width the rows were laid at.
    pub(super) fn prefetch_width(&self) -> usize {
        self.prep.width.max(1)
    }

    /// One slot's rows go, wherever they sit.
    pub(super) fn forget_num(&mut self, num: i64) {
        self.prep.rows.retain(|laid| laid.at.num != num);
    }

    /// Rows whose slot died or left the conversation go.
    pub(super) fn prune_dead(&mut self) {
        let dead: Vec<i64> = self
            .prep
            .rows
            .iter()
            .map(|laid| laid.at.num)
            .filter(|num| {
                self.local
                    .at(*num)
                    .is_some_and(|slot| slot.kind == super::local::SlotKind::Next)
            })
            .collect();
        if dead.is_empty() {
            return;
        }
        self.prep.rows.retain(|laid| !dead.contains(&laid.at.num));
    }

    /// The width changed: every laid row goes.
    pub(super) fn refold(&mut self, width: usize) {
        self.prep.rows.clear();
        self.prep.width = width;
        self.products.drop_fixed();
        self.todo_panel.dropped();
    }

    /// Keep only the pieces near the window; whole pieces leave, never half of one.
    pub(super) fn trim(&mut self, top: usize, body: usize) {
        let len = self.prep.rows.len();
        if len == 0 {
            return;
        }
        let from = top.saturating_sub(body).min(len - 1);
        let to = (top + 2 * body).min(len - 1);
        let (Some(head), Some(tail)) = (
            self.prep.rows.get(from).map(|laid| laid.at),
            self.prep.rows.get(to).map(|laid| laid.at),
        ) else {
            return;
        };
        while self.prep.rows.len() > 1 && self.prep.rows.front().map(|l| l.at) != Some(head) {
            self.prep.rows.pop_front();
        }
        while self.prep.rows.len() > 1 && self.prep.rows.back().map(|l| l.at) != Some(tail) {
            self.prep.rows.pop_back();
        }
    }
}
