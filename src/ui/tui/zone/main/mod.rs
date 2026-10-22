//! The chat zone: the host of history, input and reserved.

use super::{Ctx, Event, Handover, Request, SubZone, TermSize, Zone};

pub use arbitrate::Heights;

/// The chat zone. Sub-zones are private; their behaviour lives in their own modules.
#[derive(Default)]
pub struct MainZone {
    history: history::zone::HistoryZone,
    input: input::InputZone,
    reserved: reserved::ReservedZone,
    keys: keys::Borrowed,
    size: Option<TermSize>,
    heights: Heights,
    pending_exits: Vec<input::ExitRequest>,
    pending_requests: Vec<Request>,
}

impl MainZone {
    /// Hand the display options to the reserved zone's occupant.
    pub fn set_usage_config(&mut self, cfg: &crate::server::config::read::config::UsageConfig) {
        self.reserved.set_usage_config(cfg);
    }

    /// Hand the history zone its display options.
    pub fn set_history_config(&mut self, cfg: &history::zone::Config) {
        self.history.set_config(cfg);
    }

    /// Warm the languages the transcript uses.
    pub fn warm(&self, view: &crate::ui::session::View<'_>) {
        self.history.warm(view);
    }

    /// Where the hardware cursor goes, in terminal coordinates.
    pub fn cursor_position(&self) -> Option<(u16, u16)> {
        arbitrate::cursor_position(self)
    }
}

impl Zone for MainZone {
    fn deliver_event(&mut self, ev: Event, ctx: &Ctx) {
        self.size = Some(ctx.size);
        arbitrate::lend(self);
        self.history.deliver_event(ev, ctx);
        self.input.deliver_event(ev, ctx);
        self.reserved.deliver_event(ev, ctx);
        self.reserved.sync_usage(ctx.view);
    }

    fn deliver_action(
        &mut self,
        a: &crate::ui::tui::action::Action,
        ctx: &Ctx,
    ) -> Option<Handover> {
        self.size = Some(ctx.size);
        arbitrate::lend(self);
        dispatch::dispatch(self, a, ctx)
    }

    fn attach(&mut self, session: i64, ctx: &Ctx) {
        self.history.attach(session, ctx);
        self.input.attach(session, ctx);
        self.reserved.attach(session, ctx);
    }

    fn detach(&mut self, ctx: &Ctx) {
        self.history.detach(ctx);
        self.input.detach(ctx);
        self.reserved.detach(ctx);
        self.pending_exits.clear();
        self.pending_requests.clear();
        self.keys.clear();
    }

    fn take_over(&mut self, ctx: &Ctx) {
        self.size = Some(ctx.size);
        let Some(memo) = ctx.memo else { return };
        let Some(focus) = memo.call(crate::ui::tui::app::Operator::Read, "", "tree.focus") else {
            return;
        };
        memo.call(crate::ui::tui::app::Operator::Delete, "", "tree.focus");
        let Some((turn, block)) = focus.split_once(':') else {
            return;
        };
        let Ok(turn) = turn.parse::<i64>() else {
            return;
        };
        let Ok(block) = block.parse::<i64>() else {
            return;
        };
        self.history.focus(turn, block);
    }

    fn render(&mut self, ctx: &Ctx) -> Vec<ratatui::text::Line<'static>> {
        self.size = Some(ctx.size);
        arbitrate::settle(self);
        let palette = crate::ui::tui::theme::Palette::current();
        let theme = history::render::theme::HistoryTheme::resolve();
        let width = ctx.size.cols;
        let mut out = self.history.render_rows(ctx.view, width, &theme);
        for _ in 0..self.heights.gap {
            out.push(ratatui::text::Line::default());
        }
        out.extend(self.input.render_rows(width, &palette, ctx.view));
        self.reserved.col(self.input.cursor_col);
        out.extend(self.reserved.render_rows(width, ctx.view));
        out
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.cursor_position()
    }

    fn take_requests(&mut self) -> Vec<Request> {
        requests::take(self)
    }
}

pub mod arbitrate;
pub mod dispatch;
pub mod geometry;
pub mod history;
pub mod input;
pub mod keys;
pub mod num;
pub mod requests;
pub mod reserved;
