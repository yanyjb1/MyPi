//! The display range: where the screen starts, and the rows it takes.

use super::super::local::section::Local;
use super::super::product::{Opts, Products};
use super::super::row::{Ink, Lead, Row};
use super::segment::SegId;
use super::{Head, Placed, Run};

/// Where the screen starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// The newest rows: whatever grows lands on screen by itself.
    Latest,
    /// One fixed place: this segment, this row inside it.
    At(SegId, usize),
}

/// What one spend produced.
pub struct Spend {
    pub rows: Vec<Row>,
    pub missing: Vec<i64>,
    /// The run row the screen starts at.
    pub top: usize,
}

/// What one focus attempt found.
pub enum Aim {
    /// The position draws nothing.
    Empty,
    /// The position's content is not in hand.
    Missing(Vec<i64>),
    /// The window top, and the content missing from the rows above it.
    Top {
        top: (SegId, usize),
        missing: Vec<i64>,
    },
}

/// The display range.
#[derive(Default)]
pub struct Range {
    start: Option<Start>,
}

impl Range {
    pub fn follow(&mut self) {
        self.start = Some(Start::Latest);
    }

    pub fn start(&self) -> Start {
        self.start.unwrap_or(Start::Latest)
    }

    /// One screen: the rows, and the block ids the run lacks.
    pub fn rows(
        &mut self,
        run: &mut Run,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        height: usize,
    ) -> Spend {
        match self.start() {
            Start::Latest => latest(run, local, products, opts, height),
            Start::At(id, row) => {
                let (rows, missing, ended) = at(run, local, products, opts, height, id, row);
                if ended {
                    self.start = Some(Start::Latest);
                    return latest(run, local, products, opts, height);
                }
                Spend {
                    rows,
                    missing,
                    top: run.index_of(id, row).unwrap_or(0),
                }
            }
        }
    }

    /// Move the start `count` rows; a move past the newest follows again.
    pub fn scroll(
        &mut self,
        run: &mut Run,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        height: usize,
        older: bool,
        count: usize,
    ) -> Vec<i64> {
        let mut missing = Vec::new();
        let Some((mut id, mut row)) = self.top(run, local, products, opts, height) else {
            return missing;
        };
        let mut reached_end = false;
        for _ in 0..count.max(1) {
            let moved = if older {
                step_up(run, local, products, opts, id, row, &mut missing)
            } else {
                step_down(run, local, products, opts, id, row, &mut missing)
            };
            match moved {
                Some((next, at)) => {
                    id = next;
                    row = at;
                }
                None if !older => {
                    reached_end = true;
                    break;
                }
                None => break,
            }
        }
        self.start = Some(if reached_end {
            Start::Latest
        } else {
            Start::At(id, row)
        });
        missing
    }

    /// Where a focus would put the window, and what it still needs before landing.
    pub fn aim(
        &mut self,
        run: &mut Run,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        idx: super::super::local::section::Idx,
        focus_line: usize,
    ) -> Aim {
        match run.head(local, products, opts, idx) {
            Head::Missing(block) => Aim::Missing(vec![block]),
            Head::Empty => Aim::Empty,
            Head::Laid(id) => {
                let (top, missing) = run.back(local, products, opts, id, 0, focus_line);
                Aim::Top { top, missing }
            }
        }
    }

    /// Pin the window at an aim's top.
    pub fn pin(&mut self, top: (SegId, usize)) {
        self.start = Some(Start::At(top.0, top.1));
    }

    /// The position `rows` below the window top; the sight line's address.
    pub fn sight(
        &mut self,
        run: &mut Run,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        rows: usize,
    ) -> Option<super::super::local::section::Idx> {
        let (mut id, mut row) = match self.start() {
            Start::Latest => return None,
            Start::At(id, row) => (id, row),
        };
        let mut missing = Vec::new();
        for _ in 0..rows {
            match step_down(run, local, products, opts, id, row, &mut missing) {
                Some((next, at)) => {
                    id = next;
                    row = at;
                }
                None => break,
            }
        }
        Some(id.idx)
    }

    fn top(
        &mut self,
        run: &mut Run,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        height: usize,
    ) -> Option<(SegId, usize)> {
        match self.start() {
            Start::At(id, row) => Some((id, row)),
            Start::Latest => {
                let _ = fill_tail(run, local, products, opts, height);
                let len = tail_len(run);
                run.at(len.saturating_sub(height))
            }
        }
    }
}

/// The newest screenful: fill towards the old side, then take the last rows.
fn latest(
    run: &mut Run,
    local: &Local,
    products: &mut Products,
    opts: &Opts<'_>,
    height: usize,
) -> Spend {
    let missing = fill_tail(run, local, products, opts, height);
    let len = tail_len(run);
    let top = len.saturating_sub(height);
    let shown = len.saturating_sub(top);
    let mut rows: Vec<Row> = Vec::with_capacity(height);
    for index in top..top + shown {
        rows.push(run.row(index).cloned().unwrap_or_default());
    }
    while rows.len() < height {
        rows.push(Row::blank());
    }
    Spend { rows, missing, top }
}

/// One fixed place: lay forward from it until the screen is full.
#[allow(clippy::too_many_arguments)]
fn at(
    run: &mut Run,
    local: &Local,
    products: &mut Products,
    opts: &Opts<'_>,
    height: usize,
    id: SegId,
    row: usize,
) -> (Vec<Row>, Vec<i64>, bool) {
    let mut rows: Vec<Row> = Vec::with_capacity(height);
    let mut missing: Vec<i64> = Vec::new();
    let mut cursor = Some(id);
    let mut skip = row;
    let mut ended = false;
    while let Some(seg) = cursor {
        if rows.len() >= height {
            break;
        }
        match run.place(local, products, opts, seg) {
            Placed::Laid => {
                let Some(laid) = run.laid(seg) else {
                    break;
                };
                let from = skip.min(laid.rows.len());
                for row in &laid.rows[from..] {
                    rows.push(row.clone());
                    if rows.len() >= height {
                        break;
                    }
                }
            }
            Placed::Empty => {}
            Placed::Missing(block) => {
                missing.push(block);
                rows.push(mark());
            }
        }
        cursor = run.next(local, products, opts, seg);
        skip = 0;
        if cursor.is_none() {
            ended = true;
        }
    }
    (rows, missing, ended)
}

/// The rows a screen needs, laid towards the old side; and the block ids the run lacks.
fn fill_tail(
    run: &mut Run,
    local: &Local,
    products: &mut Products,
    opts: &Opts<'_>,
    height: usize,
) -> Vec<i64> {
    let mut missing = Vec::new();
    let Some(mut id) = run.newest(local, products, opts) else {
        return missing;
    };
    let mut need = height;
    loop {
        if let Placed::Missing(block) = run.place(local, products, opts, id) {
            missing.push(block);
        }
        let rows = run.laid(id).map_or(0, |laid| laid.rows.len());
        need = need.saturating_sub(rows);
        if need == 0 {
            return missing;
        }
        let Some(prev) = run.prev(local, products, opts, id) else {
            return missing;
        };
        id = prev;
    }
}

/// The run length without the gap that closes the newest segment.
fn tail_len(run: &Run) -> usize {
    let len = run.len();
    if len > 0 && run.row(len - 1).is_some_and(Row::is_blank) {
        len - 1
    } else {
        len
    }
}

fn step_down(
    run: &mut Run,
    local: &Local,
    products: &mut Products,
    opts: &Opts<'_>,
    id: SegId,
    row: usize,
    missing: &mut Vec<i64>,
) -> Option<(SegId, usize)> {
    let rows = match run.place(local, products, opts, id) {
        Placed::Laid => run.laid(id).map_or(0, |laid| laid.rows.len()),
        Placed::Missing(block) => {
            missing.push(block);
            1
        }
        Placed::Empty => 0,
    };
    if row + 1 < rows {
        return Some((id, row + 1));
    }
    let mut cursor = run.next(local, products, opts, id);
    while let Some(next) = cursor {
        match run.place(local, products, opts, next) {
            Placed::Laid => return Some((next, 0)),
            Placed::Missing(block) => {
                missing.push(block);
                return Some((next, 0));
            }
            Placed::Empty => cursor = run.next(local, products, opts, next),
        }
    }
    None
}

fn step_up(
    run: &mut Run,
    local: &Local,
    products: &mut Products,
    opts: &Opts<'_>,
    id: SegId,
    row: usize,
    missing: &mut Vec<i64>,
) -> Option<(SegId, usize)> {
    if row > 0 {
        return Some((id, row - 1));
    }
    let mut cursor = run.prev(local, products, opts, id);
    while let Some(prev) = cursor {
        match run.place(local, products, opts, prev) {
            Placed::Laid => {
                let rows = run.laid(prev).map_or(0, |laid| laid.rows.len());
                return Some((prev, rows.saturating_sub(1)));
            }
            Placed::Missing(block) => {
                missing.push(block);
                return Some((prev, 0));
            }
            Placed::Empty => cursor = run.prev(local, products, opts, prev),
        }
    }
    None
}

fn mark() -> Row {
    Row::text(
        Lead::None,
        "…",
        Some(Ink::Fixed(crate::ui::tui::theme::fixed::SYSTEM)),
    )
}
