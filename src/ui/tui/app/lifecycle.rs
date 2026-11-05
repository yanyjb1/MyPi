//! Session attachment: the data first, then the zone that shows.

use super::App;

impl App {
    /// Enter a conversation: follow it, watch it, then tell the zone that shows.
    pub fn attach(&mut self, id: i64) {
        if self.attached == Some(id) {
            return;
        }
        self.session.attach(id);
        self.session.watch(id);
        self.attached = Some(id);
        self.reset_memo();
        for told in &mut self.told {
            *told = None;
        }
        self.tell();
    }

    /// Leave the conversation. The zone that shows hears it.
    pub fn detach(&mut self) {
        self.session.detach();
        self.attached = None;
        self.reset_memo();
        for told in &mut self.told {
            *told = None;
        }
        self.tell();
    }

    /// The zone that shows is the only one told about the attachment, and only once.
    pub(super) fn tell(&mut self) {
        let Some(owner) = self.owner else {
            return;
        };
        let index = owner.index();
        if self.told[index] == self.attached {
            return;
        }
        self.told[index] = self.attached;
        let requests = self.zones[index].session(self.attached);
        self.land_all(requests);
    }
}
