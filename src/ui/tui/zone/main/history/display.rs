//! The display layer: where the reader sits, and how one scroll moves them.

use ratatui::text::Line;

use super::layout::line::Row;
use super::local::SlotKind;
use super::prefetch::Anchor;
use super::zone::HistoryZone;

use crate::ui::tui::theme::Accent;

/// Which way the reader moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Older,
    Newer,
}

/// The window's own state: screen rows above the sight line, the rows the host
/// allocated, and the row the window starts on. `None` = follow the newest.
pub(crate) struct Display {
    line: u16,
    rows: u16,
    at: Option<Anchor>,
}

impl Default for Display {
    fn default() -> Self {
        Display {
            line: 3,
            rows: 0,
            at: None,
        }
    }
}

impl Display {
    pub(super) fn set_line(&mut self, line: u16) {
        self.line = line.max(1);
    }

    pub(super) fn assign(&mut self, rows: u16) {
        self.rows = rows;
    }

    pub(super) fn rows(&self) -> u16 {
        self.rows
    }

    pub(super) fn follow(&mut self) {
        self.at = None;
    }

    pub(super) fn top(&self) -> Option<Anchor> {
        self.at
    }

    pub(super) fn set_top(&mut self, at: Option<Anchor>) {
        self.at = at;
    }

    /// Rows above the sight line.
    pub(super) fn line(&self) -> u16 {
        self.line
    }
}

impl HistoryZone {
    /// One frame: the window from the prefetch layer, then the checklist.
    pub fn render_rows(&mut self, term_w: u16, accent: Accent) -> Vec<Line<'static>> {
        let width = usize::from(term_w.max(1));
        if self.prep.width != width {
            self.refold(width);
        }
        let pinned = self.todo_rows(width);
        let body = usize::from(self.disp.rows()).saturating_sub(pinned.len());
        let top = self.settle_window(body);
        let mut out: Vec<Line<'static>> = Vec::with_capacity(usize::from(self.disp.rows()));
        for offset in 0..body {
            match self.prep.rows.get(top + offset) {
                Some(laid) => out.push(laid.row.paint(accent)),
                None => out.push(Line::default()),
            }
        }
        out.extend(pinned.into_iter().map(|row| row.paint(accent)));
        self.trim(top, body);
        out
    }

    /// The checklist's rows, kept by its own product.
    fn todo_rows(&mut self, width: usize) -> Vec<Row> {
        let HistoryZone {
            todo,
            todo_panel,
            todo_fold,
            disp,
            ..
        } = self;
        let budget = usize::from(disp.rows()) / 2;
        todo_panel.rows(todo, *todo_fold, width, budget).to_vec()
    }

    /// Scrolling presupposes a run that at least fills the window.
    fn scrollable(&mut self) -> bool {
        let body = usize::from(self.disp.rows()).max(1);
        let _ = self.settle_window(body);
        self.prep.rows.len() >= body
    }

    /// Where the window starts this frame.
    fn settle_window(&mut self, body: usize) -> usize {
        let body = body.max(1);
        self.prune_dead();
        let Some(at) = self.disp.top() else {
            while self.extend_newer() {}
            while self.prep.rows.len() < body && self.extend_older() {}
            return self.prep.rows.len().saturating_sub(body);
        };
        if self.index_of(at).is_none() {
            self.disp.follow();
            while self.extend_newer() {}
            return self.prep.rows.len().saturating_sub(body);
        }
        // A frozen window still fills the screen: the run grows towards the past first.
        while self.prep.rows.len() < body && self.extend_older() {}
        let top = self.index_of(at).unwrap_or(0);
        while self.prep.rows.len() < top + body && self.extend_newer() {}
        // The window never starts past the row that still fills it from below.
        top.min(self.prep.rows.len().saturating_sub(body))
    }

    /// One scroll: screen rows, or a whole page. Nothing moves unless the run is taller than
    /// the window, and the bottom sticks.
    pub(super) fn scroll(&mut self, dir: Dir, rows: usize) {
        if !self.scrollable() {
            return;
        }
        let body = usize::from(self.disp.rows()).max(1);
        let cur = self.settle_window(body) as i64;
        let step = rows.max(1) as i64;
        let mut want = match dir {
            Dir::Older => cur - step,
            Dir::Newer => cur + step,
        };
        let mut grew_older = false;
        while want < 0 {
            let before = self.prep.rows.len();
            if !self.extend_older() {
                want = 0;
                break;
            }
            let added = (self.prep.rows.len() - before) as i64;
            if added == 0 {
                want = 0;
                break;
            }
            grew_older = true;
            want += added;
        }
        if dir == Dir::Older && !grew_older && cur == 0 {
            // Nothing above and the window already starts there: nothing moves.
            return;
        }
        let mut stuck = false;
        while want as usize >= self.prep.rows.len() {
            if !self.extend_newer() {
                stuck = true;
                break;
            }
        }
        let last_top = self.prep.rows.len().saturating_sub(body);
        if dir == Dir::Newer && (stuck || want as usize >= last_top) {
            self.disp.follow();
            return;
        }
        let index = (want.max(0) as usize).min(last_top);
        if let Some(at) = self.anchor_of(index) {
            self.disp.set_top(Some(at));
        }
    }

    /// One whole slot older or newer.
    pub(super) fn step(&mut self, dir: Dir) {
        if !self.scrollable() {
            return;
        }
        let from = self
            .disp
            .top()
            .map(|at| at.num)
            .or_else(|| self.local.newest())
            .unwrap_or(0);
        let found = match dir {
            Dir::Older => {
                let Some(oldest) = self.local.oldest() else {
                    return;
                };
                let mut num = from - 1;
                let mut found = None;
                while num >= oldest {
                    if self.piece_count(num) > 0 {
                        found = Some(num);
                        break;
                    }
                    num -= 1;
                }
                found
            }
            Dir::Newer => {
                let tail = self.tail_num();
                let mut num = from + 1;
                let mut found = None;
                while num <= tail {
                    if self.piece_count(num) > 0 {
                        found = Some(num);
                        break;
                    }
                    num += 1;
                }
                found
            }
        };
        match found {
            Some(num) => self.disp.set_top(Some(Anchor { num, row: 0 })),
            None if dir == Dir::Newer => self.disp.follow(),
            None => {}
        }
    }

    /// One whole round older or newer: its first block under the sight line.
    pub(super) fn turn_step(&mut self, dir: Dir) {
        if !self.scrollable() {
            return;
        }
        let anchor = self.anchor_num();
        let Some(block) = self.local.at(anchor).and_then(|slot| match slot.kind {
            SlotKind::Block(id) => Some(id),
            _ => None,
        }) else {
            return;
        };
        let Some(turn) = self.turn_of(block) else {
            return;
        };
        let rounds = self.structure.rounds();
        let Some(at) = rounds.iter().position(|round| round.id == turn) else {
            return;
        };
        let next = match dir {
            Dir::Older => at + 1,
            Dir::Newer => match at.checked_sub(1) {
                Some(index) => index,
                None => return,
            },
        };
        let Some([lo, _]) = rounds.get(next).and_then(|round| round.ranges.first()) else {
            return;
        };
        let Some(num) = self.local.num_of(*lo) else {
            return;
        };
        self.focus_num(num);
    }

    /// Put one slot's head under the sight line, the line itself left clear.
    pub(super) fn focus_num(&mut self, num: i64) {
        let line = self.sight();
        let Some(mut head) = self.head_of(num) else {
            return;
        };
        // Fill in above until the head can sit below the sight line.
        while head < line {
            let before = self.prep.rows.len();
            if !self.extend_older() || self.prep.rows.len() == before {
                break;
            }
            head = self.head_in(num).unwrap_or(head);
        }
        let want = head.saturating_sub(line);
        self.disp.set_top(self.anchor_of(want));
    }

    /// The sight line's own row; `focusLine` counts screen rows from one.
    fn sight(&self) -> usize {
        usize::from(self.disp.line()).saturating_sub(1)
    }

    /// The slot under the sight line; the newest when following.
    pub(super) fn anchor_num(&mut self) -> i64 {
        let Some(at) = self.disp.top() else {
            return self.local.newest().unwrap_or(0);
        };
        let line = self.sight();
        let Some(top) = self.index_of(at) else {
            return at.num;
        };
        self.anchor_of(top + line).map(|a| a.num).unwrap_or(at.num)
    }

    /// The oldest slot under the sight line.
    pub(super) fn first(&mut self) {
        if !self.scrollable() {
            return;
        }
        let Some(num) = self.local.oldest() else {
            return;
        };
        self.focus_num(num);
    }

    /// Put the sight line on the first or last block of the anchor's turn.
    pub(super) fn align(&mut self, last: bool) {
        if !self.scrollable() {
            return;
        }
        let anchor = self.anchor_num();
        let Some(id) = self.local.at(anchor).and_then(|slot| match slot.kind {
            SlotKind::Block(id) => Some(id),
            _ => None,
        }) else {
            return;
        };
        let Some(turn) = self.turn_of(id) else {
            return;
        };
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
            self.focus_num(*num);
        }
    }

    /// The host gave the zone new rows.
    pub(super) fn resize(&mut self, rows: u16) {
        self.disp.assign(rows);
    }
}
