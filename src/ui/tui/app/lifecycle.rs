//! Session attachment: the data first, then the zone that shows.

use super::App;

impl App {
    /// Enter a conversation: follow it, watch it, then tell the zone that shows.
    pub fn attach(&mut self, id: i64) {
        if self.attached == Some(id) {
            return;
        }
        self.session.attach(id);
        // A resumed conversation stands where it was started, whatever row it carried.
        let cwd = self.cwd.to_string_lossy().to_string();
        self.session.set_cwd(&cwd);
        self.session.watch(id);
        self.attached = Some(id);
        self.profile = None;
        self.name = None;
        self.reset_memo();
        self.tell();
    }

    /// Leave the conversation. The zone that shows hears it.
    pub fn detach(&mut self) {
        self.session.detach();
        self.attached = None;
        self.profile = None;
        self.name = None;
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
        let requests = self.zones[index].session(self.attached);
        self.land_all(requests);
    }
}
