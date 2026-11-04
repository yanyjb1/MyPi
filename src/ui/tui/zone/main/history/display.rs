//! The display layer: where the reader sits, and how one scroll moves them.

use ratatui::text::Line;

use super::layout::line::Row;
use super::local::SlotKind;
use super::prefetch::Origin;
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
    at: Option<Origin>,
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

    pub(super) fn top(&self) -> Option<Origin> {
        self.at
    }

    pub(super) fn set_top(&mut self, at: Option<Origin>) {
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

    /// Where the window starts this frame.
    fn settle_window(&mut self, body: usize) -> usize {
        let body = body.max(1);
        self.prune_dead();
        match self.disp.top() {
            None => {
                while self.extend_newer() {}
                self.prep.rows.len().saturating_sub(body)
            }
            Some(at) => {
                let top = match self.prep.rows.iter().position(|laid| laid.at == at) {
                    Some(index) => index,
                    None => {
                        self.lay_from(at);
                        0
                    }
                };
                while self.prep.rows.len() < top + body && self.extend_newer() {}
                top.min(self.prep.rows.len().saturating_sub(1))
            }
        }
    }

    /// One scroll: screen rows, or a whole page. Reaching the end follows again.
    pub(super) fn scroll(&mut self, dir: Dir, rows: usize) {
        let body = usize::from(self.disp.rows()).max(1);
        let cur = self.settle_window(body) as i64;
        let step = rows.max(1) as i64;
        let mut want = match dir {
            Dir::Older => cur - step,
            Dir::Newer => cur + step,
        };
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
            want += added;
        }
        let mut stuck = false;
        while want as usize >= self.prep.rows.len() {
            if !self.extend_newer() {
                stuck = true;
                break;
            }
        }
        if dir == Dir::Newer && stuck {
            self.disp.follow();
            return;
        }
        let index = (want.max(0) as usize).min(self.prep.rows.len().saturating_sub(1));
        if let Some(laid) = self.prep.rows.get(index) {
            let at = laid.at;
            self.disp.set_top(Some(at));
        }
    }

    /// One whole slot older or newer.
    pub(super) fn step(&mut self, dir: Dir) {
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
            Some(num) => self.disp.set_top(Some(Origin { num, piece: 0 })),
            None if dir == Dir::Newer => self.disp.follow(),
            None => {}
        }
    }

    /// Put one slot's head under the sight line.
    pub(super) fn focus_num(&mut self, num: i64) {
        self.disp.set_top(Some(Origin { num, piece: 0 }));
        let line = usize::from(self.disp.line());
        if line > 0 {
            self.scroll(Dir::Older, line);
        }
    }

    /// The oldest slot under the sight line; the newest when following.
    pub(super) fn anchor_num(&self) -> i64 {
        self.disp
            .top()
            .map(|at| at.num)
            .or_else(|| self.local.newest())
            .unwrap_or(0)
    }

    /// The oldest slot under the sight line.
    pub(super) fn first(&mut self) {
        let Some(num) = self.local.oldest() else {
            return;
        };
        self.focus_num(num);
    }

    /// Put the sight line on the first or last block of the anchor's turn.
    pub(super) fn align(&mut self, last: bool) {
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
