//! The handover transaction: memo window, request carry, ownership.

use super::App;
use crate::ui::tui::zone::{Ctx, Handover, Zone, ZoneId};

impl App {
    /// Execute one handover. The old zone has already put its queue in `h.requests`.
    pub fn hand_over(&mut self, h: Handover, ctx: &Ctx) {
        let old = self.ownership.showing();
        let to = h.to;
        let memo_ctx = Ctx {
            view: ctx.view,
            size: ctx.size,
            memo: Some(&self.memo),
        };

        match old {
            Some(ZoneId::Main) => self.main.hand_over(&memo_ctx),
            Some(ZoneId::Resume) => self.resume.hand_over(&memo_ctx),
            Some(ZoneId::Tree) => self.tree.hand_over(&memo_ctx),
            None => {}
        }

        self.carried.extend(h.requests);
        self.ownership.register(to);

        match to {
            Some(ZoneId::Main) => self.main.take_over(&memo_ctx),
            Some(ZoneId::Resume) => self.resume.take_over(&memo_ctx),
            Some(ZoneId::Tree) => self.tree.take_over(&memo_ctx),
            None => {}
        }

        debug_assert!(
            match old {
                Some(ZoneId::Main) => self.main.take_requests().is_empty(),
                Some(ZoneId::Resume) => self.resume.take_requests().is_empty(),
                Some(ZoneId::Tree) => self.tree.take_requests().is_empty(),
                None => true,
            },
            "the old zone kept requests after handing the screen over"
        );
    }
}
