//! Session attachment: the connection first, then the zone that shows.

use serde_json::json;

use super::App;
use crate::ui::tui::zone::{Frame, ZoneId};

impl App {
    /// Enter a conversation: follow it, watch it, then tell the zone that shows.
    pub fn attach(&mut self, id: i64) {
        if self.attached == Some(id) {
            return;
        }
        self.session.attach(id);
        self.session
            .send(Frame::new("subscribe", "watch", json!({"session": id})));
        self.flush();
        self.attached = Some(id);
        self.zones[ZoneId::Main.index()].draft_cleared();
        self.reset_memo();
        self.tell();
    }

    /// Leave the conversation: the queued message goes, then the watch.
    pub fn detach(&mut self) {
        self.flush();
        if let Some(id) = self.attached {
            self.session
                .send(Frame::new("post", "mailbox", json!({"session": id})));
            self.asking.push_back(None);
        }
        self.session.detach();
        self.session
            .send(Frame::new("subscribe", "watch", json!({"session": null})));
        self.attached = None;
        self.reset_memo();
        self.tell();
    }

    /// The zone that shows is the only one told about the attachment, and only when it changed.
    pub(super) fn tell(&mut self) {
        let Some(owner) = self.owner else {
            return;
        };
        let index = owner.index();
        if self.told[index] == Some(self.attached) {
            return;
        }
        self.told[index] = Some(self.attached);
        let locals = self.zones[index].session(self.attached);
        self.land_all(locals);
    }
}
