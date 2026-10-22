//! The block tree: a full-screen navigator over the blocks of one conversation.

pub mod ask;
pub mod layout;
mod render;
pub mod rows;
pub mod source;

pub use rows::Row;

use crate::ui::tui::action::{Action, KeyCode};
use crate::ui::tui::app::Operator;
use crate::ui::tui::zone::{Ctx, Event, Handover, Request, TermSize, Zone, ZoneId};

#[derive(Default)]
pub struct TreeZone {
    size: Option<TermSize>,
    rows: Vec<Row>,
    pub selected: usize,
    pub scroll: usize,
    pending: Vec<Request>,
    only_user: bool,
    focus_on_handover: bool,
    requested: bool,
    cherish: bool,
}

impl TreeZone {
    pub fn begin(&mut self, rows: Vec<Row>, current: usize) {
        self.rows = rows;
        self.selected = current.min(self.rows.len().saturating_sub(1));
        self.scroll = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// `cherishYourWords`: Enter becomes focus, Shift+Enter becomes rewind.
    pub fn set_config(&mut self, cherish: bool) {
        self.cherish = cherish;
    }

    /// Rebuild rows, keeping the selected block where possible.
    fn sync(&mut self, view: &crate::ui::session::View<'_>) {
        let keep = self.rows.get(self.selected).map(|row| row.block_id);
        let rows = source::rows(view, self.only_user);
        self.rows = rows;
        if let Some(id) = keep
            && let Some(at) = self.rows.iter().position(|row| row.block_id == id)
        {
            self.selected = at;
        } else {
            self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(1));
        self.request_range(view);
    }

    fn move_by(&mut self, delta: i64) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        let next = self.selected as i64 + delta;
        self.selected = next.clamp(0, last as i64) as usize;
    }

    /// Enter: ask for a rewind to the highlighted block.
    fn pick(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        self.pending.push(Request::Rewind {
            turn: row.turn,
            block: row.block_id,
            text: row.body.clone(),
        });
    }

    /// Ask the session for the whole shaped span once per takeover.
    fn request_range(&mut self, view: &crate::ui::session::View<'_>) {
        if self.requested {
            return;
        }
        if let Some((start, end)) = source::range(view) {
            self.pending.push(Request::TreeRange { start, end });
            self.requested = true;
        }
    }

    fn handover(&mut self) -> Handover {
        Handover {
            to: Some(ZoneId::Main),
            requests: self.take_requests(),
        }
    }
}

impl Zone for TreeZone {
    fn deliver_event(&mut self, ev: Event, ctx: &Ctx) {
        self.size = Some(ctx.size);
        if ev == Event::Resized {
            self.scroll = 0;
        }
        self.sync(ctx.view);
    }

    fn deliver_action(&mut self, a: &Action, ctx: &Ctx) -> Option<Handover> {
        self.size = Some(ctx.size);
        let Action::Key(key) = a else {
            return None;
        };
        match key.code {
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.rows.len().saturating_sub(1),
            KeyCode::Char('s') if !key.mods.ctrl && !key.mods.alt => {
                self.only_user = !self.only_user;
                self.sync(ctx.view);
            }
            KeyCode::Enter => {
                let rewind = if self.cherish {
                    key.mods.shift
                } else {
                    !key.mods.shift
                };
                if rewind {
                    self.pick();
                } else {
                    self.focus_on_handover = true;
                }
                return Some(self.handover());
            }
            KeyCode::Esc | KeyCode::Char('q') => return Some(self.handover()),
            _ => {}
        }
        None
    }

    fn take_over(&mut self, ctx: &Ctx) {
        self.size = Some(ctx.size);
        self.sync(ctx.view);
        let current = source::current(ctx.view, &self.rows);
        self.begin(std::mem::take(&mut self.rows), current);
        self.request_range(ctx.view);
    }

    fn hand_over(&mut self, ctx: &Ctx) {
        if !self.focus_on_handover {
            return;
        }
        self.focus_on_handover = false;
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        if let Some(memo) = ctx.memo {
            memo.call(
                Operator::Write,
                &format!("{}:{}", row.turn, row.block_id),
                "tree.focus",
            );
        }
    }

    fn detach(&mut self, _ctx: &Ctx) {
        self.size = None;
        self.rows.clear();
        self.selected = 0;
        self.scroll = 0;
        self.pending.clear();
        self.only_user = false;
        self.focus_on_handover = false;
        self.requested = false;
    }

    fn render(&mut self, ctx: &Ctx) -> Vec<ratatui::text::Line<'static>> {
        self.size = Some(ctx.size);
        self.sync(ctx.view);
        let theme = crate::ui::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        render::rows(
            &self.rows,
            self.selected,
            Some(ctx.size),
            &mut self.scroll,
            &theme,
        )
    }

    fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.pending)
    }
}
