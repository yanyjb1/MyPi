//! The rows of one frame: the cache, the window, and the accent at the end.

use std::collections::BTreeMap;

use ratatui::text::Line;

use super::cache::{Cache, Class};
use super::line::{Ink, Lead, Row};
use super::local::{Local, SlotKind};
use super::paint::{self, Opts};
use super::segment::{self, Kind};
use super::shape::{Block, Kind as BlockKind};
use super::view::{Anchor, Dir, Seam, Spot};
use super::zone::HistoryZone;

use crate::ui::tui::theme::{Accent, fixed};

impl HistoryZone {
    /// The rows of one frame: the conversation, then the pinned rows.
    pub fn render_rows(&mut self, term_w: u16, accent: Accent) -> Vec<Line<'static>> {
        let width = usize::from(term_w.max(1));
        self.cache
            .sync(width, (self.thinking, self.tool), self.fold);
        let placeholder = self.placeholder.clone();
        let cwd = self.cwd.clone();
        let opts = Opts {
            thinking: self.thinking,
            tool: self.tool,
            placeholder: &placeholder,
            cwd: &cwd,
            fold: self.fold,
        };

        let pinned = paint::todo::rows(
            &self.todo,
            self.todo_fold,
            width,
            usize::from(self.rows) / 2,
        );
        let tail = self.live.rows(&opts, width);
        let mid = match self.view.seam() {
            Seam::End => 0,
            Seam::At(anchor) => self.seam_row(anchor),
        };

        let Self {
            blocks,
            local,
            view,
            cache,
            ..
        } = self;
        let rows = view.compose(
            local,
            |num| slot_rows(blocks, cache, local, num, &opts, width),
            mid,
            &tail,
            &pinned,
        );
        rows.iter().map(|row| row.paint(accent)).collect()
    }

    /// Where a stored spot falls at the current width.
    fn seam_row(&self, anchor: Anchor) -> usize {
        match anchor.spot {
            Spot::Row(row) => row,
            Spot::Natural { line, off } => {
                let Some(id) = self.local.at(anchor.num).and_then(|slot| match slot.kind {
                    SlotKind::Block(id) => Some(id),
                    _ => None,
                }) else {
                    return 0;
                };
                self.cache.row_of(id, line, off).unwrap_or(0)
            }
        }
    }

    /// A new row budget: the natural line stays, the row inside it is recomputed.
    pub(super) fn resize(&mut self, rows: u16) {
        self.rows = rows;
        self.view.assign(rows);
        let Seam::At(anchor) = self.view.seam() else {
            return;
        };
        let Some(slot) = self.local.at(anchor.num) else {
            self.view.follow();
            return;
        };
        if slot.kind == SlotKind::Next {
            self.view.follow();
            return;
        }
        let spot = match (anchor.spot, &slot.kind) {
            (Spot::Natural { .. }, _) => anchor.spot,
            (Spot::Row(row), SlotKind::Block(id)) => {
                let (line, off) = self.cache.spot_of(*id, row);
                Spot::Natural { line, off }
            }
            (Spot::Row(_), _) => Spot::Row(0),
        };
        self.view.set_seam(Seam::At(Anchor {
            num: anchor.num,
            spot,
        }));
    }

    /// A screen older or newer, or one slot older or newer.
    pub(super) fn scroll(&mut self, dir: Dir, page: bool) {
        if page {
            let heights = &self.cache;
            let local = &self.local;
            let mut view = std::mem::take(&mut self.view);
            view.page(
                local,
                |num| {
                    local
                        .at(num)
                        .and_then(|slot| match slot.kind {
                            SlotKind::Block(id) => heights.height(id),
                            _ => None,
                        })
                        .unwrap_or(1)
                },
                dir,
            );
            self.view = view;
        } else {
            self.view.step(&self.local, dir);
        }
        self.ensure_around();
    }

    /// Align the sight line with the first or last block of the anchor's turn.
    pub(super) fn align(&mut self, last: bool) {
        let anchor = self.view.anchor(&self.local);
        let Some(id) = self.local.at(anchor).and_then(|slot| match slot.kind {
            SlotKind::Block(id) => Some(id),
            _ => None,
        }) else {
            return;
        };
        let Some(turn) = self.turn_of(id) else { return };
        let nums: Vec<i64> = self
            .local
            .iter()
            .filter(|slot| match slot.kind {
                SlotKind::Block(id) => self.turn_of(id) == Some(turn),
                _ => false,
            })
            .map(|slot| slot.num)
            .collect();
        let num = if last {
            nums.iter().max()
        } else {
            nums.iter().min()
        };
        if let Some(num) = num {
            self.view.focus_num(*num);
            self.ensure_around();
        }
    }
}

/// One slot's display rows.
fn slot_rows(
    blocks: &BTreeMap<i64, Block>,
    cache: &mut Cache,
    local: &Local,
    num: i64,
    opts: &Opts<'_>,
    width: usize,
) -> Vec<Row> {
    let Some(slot) = local.at(num) else {
        return Vec::new();
    };
    match &slot.kind {
        SlotKind::Next => Vec::new(),
        SlotKind::Notice { text, level, .. } => paint::notice::block(text, *level),
        SlotKind::Block(id) => block_rows(blocks, cache, *id, opts, width),
    }
}

fn block_rows(
    blocks: &BTreeMap<i64, Block>,
    cache: &mut Cache,
    id: i64,
    opts: &Opts<'_>,
    width: usize,
) -> Vec<Row> {
    let Some(block) = blocks.get(&id) else {
        return vec![Row::text(Lead::None, "…", Some(Ink::Fixed(fixed::SYSTEM)))];
    };
    match block.kind {
        BlockKind::ToolResult => Vec::new(),
        BlockKind::ToolRequest => {
            let paired = blocks.get(&(id + 1)).filter(|next| block.answers(next));
            cache
                .slot(id, Class::Whole, Vec::new, || {
                    paint::card::node(block, paired, opts, width)
                })
                .rows
                .clone()
        }
        BlockKind::User | BlockKind::Wake => cache
            .slot(id, Class::Whole, Vec::new, || {
                paint::user::block(block, width)
            })
            .rows
            .clone(),
        BlockKind::Assistant | BlockKind::Reasoning => {
            if block.kind == BlockKind::Reasoning && !opts.thinking {
                return vec![paint::prose::placeholder(opts.placeholder)];
            }
            let text = block
                .content
                .as_deref()
                .or(block.text.as_deref())
                .unwrap_or_default();
            let rows = if has_table(text) {
                paint::prose::inline(text, width)
            } else {
                cache
                    .slot(id, Class::Markdown, || units(text, width), Vec::new)
                    .rows
                    .clone()
            };
            if block.kind == BlockKind::Reasoning {
                paint::prose::thinking(rows)
            } else {
                rows
            }
        }
    }
}

/// A table's columns depend on the width, so such a block skips the parsed layer.
fn has_table(text: &str) -> bool {
    segment::split(text)
        .iter()
        .any(|seg| seg.kind == Kind::Table)
}

fn units(text: &str, width: usize) -> Vec<Vec<Row>> {
    segment::split(text)
        .into_iter()
        .map(|seg| match seg.kind {
            Kind::Code => paint::code::rows(&seg),
            Kind::Table => paint::table::rows(&seg, width),
            _ => paint::prose::text(&seg),
        })
        .collect()
}
