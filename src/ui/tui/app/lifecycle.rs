//! Session attachment: data first, display broadcast second.

use super::App;
use crate::ui::tui::zone::{Ctx, Zone};

impl App {
    pub fn session(&self) -> Option<i64> {
        self.session
    }

    /// Enter a conversation. A different id leaves the old one first.
    pub fn attach(&mut self, id: i64, ctx: &Ctx) {
        if self.session == Some(id) {
            return;
        }
        if self.session.is_some() {
            self.detach(ctx);
        }
        self.memo.clear();
        self.session = Some(id);
        self.main.attach(id, ctx);
        self.resume.attach(id, ctx);
        self.tree.attach(id, ctx);
        self.main.warm(ctx.view);
    }

    /// Leave the conversation. Every zone hears it, attached or not.
    pub fn detach(&mut self, ctx: &Ctx) {
        self.main.detach(ctx);
        self.resume.detach(ctx);
        self.tree.detach(ctx);
        self.session = None;
        self.memo.clear();
        self.carried.clear();
    }
}
