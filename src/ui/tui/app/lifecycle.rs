//! Session attachment: data first, the zones' broadcast second.

use super::App;
use crate::ui::tui::zone::Kind;

impl App {
    /// Enter a conversation: follow it, watch it, then tell every zone.
    pub fn attach(&mut self, id: i64) {
        if self.attached == Some(id) {
            return;
        }
        self.session.attach(id);
        self.session.send(
            Kind::Subscribe.as_str(),
            "watch",
            serde_json::json!({"session": id}),
            false,
        );
        self.attached = Some(id);
        self.reset_memo();
        let mut requests = Vec::new();
        for zone in &mut self.zones {
            requests.extend(zone.session(Some(id)));
        }
        self.land_all(requests);
    }

    /// Leave the conversation. Every zone hears it.
    pub fn detach(&mut self) {
        self.session.detach();
        self.attached = None;
        self.reset_memo();
        let mut requests = Vec::new();
        for zone in &mut self.zones {
            requests.extend(zone.session(None));
        }
        self.land_all(requests);
    }
}
