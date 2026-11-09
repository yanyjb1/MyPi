//! Enter: a line leaves as a turn, a command, or a sentence in the mailbox.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::command::{self, Call, Done};
use super::exit::ExitRequest;
use super::notice::Notice;
use super::queue::Outgoing;
use super::route::{self, Route};
use super::zone::InputZone;
use crate::ui::command::CommandMeta;
use crate::ui::tui::zone::{Request, Shout, name};

/// Two Enter presses this close together: the second one interrupts the round.
const ENTER_WINDOW: Duration = Duration::from_millis(600);

/// One notice waiting for the receipt that says its write landed.
pub(super) struct Waiting {
    pub door: &'static str,
    pub notice: Notice,
}

impl InputZone {
    /// Enter: the box's text leaves by the route it names.
    pub(super) fn enter(&mut self) -> bool {
        let text = self.editor.expanded_text();
        if text.trim().is_empty() {
            // Only a running round has the double tap: the second Enter interrupts.
            if self.data.busy
                && self
                    .enter_armed
                    .take()
                    .is_some_and(|at| at.elapsed() < ENTER_WINDOW)
            {
                self.pending_exits.push(ExitRequest::Interrupt);
                return true;
            }
            return false;
        }
        match route::classify(&text) {
            Route::Command(meta, args) => {
                self.run(meta, args);
                true
            }
            Route::Message if self.data.busy => {
                self.enqueue(text);
                true
            }
            Route::Message => {
                self.send(text);
                true
            }
        }
    }

    /// One command line: run it, then take the box away unless it asks to stay.
    fn run(&mut self, meta: &'static CommandMeta, args: &str) {
        let call = Call {
            args,
            session: self.session,
        };
        match command::run(meta, &call) {
            Done::Write { request, notice } => self.wrote(request, notice),
            Done::Exit(exit) => self.pending_exits.push(exit),
            Done::Hire(who) => self.pending_hire = Some(who),
            Done::Profile(profile) => self.outbox.push(Request::PickProfile(profile)),
            Done::Notice(notice) => self.say(&notice),
            Done::Hold => return,
        }
        self.take_line();
    }

    /// Take the line out of the box: it is history now, and the box is idle.
    fn take_line(&mut self) {
        self.history.push(self.editor.text());
        self.editor.clear();
        self.finished = true;
    }

    /// One write leaves: its notice waits for the receipt of that door.
    fn wrote(&mut self, request: Request, notice: Notice) {
        if let Request::Rename(name) = &request {
            self.pending_name = Some(name.clone());
        }
        match door_of(&request) {
            Some(door) => self.pending_notice.push_back(Waiting { door, notice }),
            None => self.say(&notice),
        }
        self.outbox.push(request);
    }

    /// One line for the history zone.
    pub(super) fn say(&mut self, notice: &Notice) {
        let text = notice.text(self.data.lang);
        self.shouts.push(Shout::new(
            name::NOTICE,
            json!({"level": notice.level(), "text": text}),
        ));
    }

    /// One write answered: the notice speaks only if the write landed. A draft's default
    /// model is the one it just wrote, so the pill follows the notice.
    pub(super) fn receipt(&mut self, door: &str, content: &Value, out: &mut Vec<Request>) {
        let Some(at) = self
            .pending_notice
            .iter()
            .position(|wait| wait.door == door)
        else {
            return;
        };
        let Some(wait) = self.pending_notice.remove(at) else {
            return;
        };
        if content.get("error").is_some() {
            return;
        }
        if let Notice::Default(id) = &wait.notice
            && self.session.is_none()
        {
            let address = format!("{}:{}", id.provider, id.model);
            self.model(&address, out);
        }
        self.say(&wait.notice);
    }

    /// A sentence sent while a round runs: the slot takes it, the round is untouched.
    fn enqueue(&mut self, text: String) {
        self.take_line();
        if let Some(request) = self.queue.push(Outgoing::new(text)) {
            self.outbox.push(request);
        }
        self.shouts
            .push(Shout::new(name::QUEUE, json!({"count": self.queue.len()})));
        self.enter_armed = Some(Instant::now());
    }

    /// A sentence sent while nothing runs: it opens a turn.
    fn send(&mut self, text: String) {
        self.pending_exits.push(ExitRequest::Submit(text.clone()));
        self.in_flight = Some(text);
        self.take_line();
        self.data.busy = true;
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": true})));
    }

    /// The reply to a submit that opened nothing: the sentence comes back, and says why.
    pub(super) fn refused(&mut self, content: &Value) {
        let reason = content
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("the session is still in a turn")
            .to_string();
        self.shouts
            .push(Shout::new(name::RETRACT, json!({"reason": reason})));
        if let Some(text) = self.in_flight.take() {
            self.prefill(&text);
        }
    }

    /// The slot became a turn: the head left the queue, and the next one takes the slot.
    pub(super) fn opened(&mut self, content: &Value) -> Vec<Request> {
        let Some(turn) = content.get("turn").and_then(Value::as_i64) else {
            return Vec::new();
        };
        if self.last_mailbox_turn == Some(turn) {
            return Vec::new();
        }
        self.last_mailbox_turn = Some(turn);
        // A turn just opened: the box is busy before its first piece arrives.
        self.data.busy = true;
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": true})));
        let (taken, request) = self.queue.took();
        let out: Vec<Request> = request.into_iter().collect();
        self.shouts
            .push(Shout::new(name::QUEUE, json!({"count": self.queue.len()})));
        if let Some(msg) = taken {
            self.shouts.push(Shout::new(
                name::ECHO,
                json!({"text": msg.text, "prefix": msg.prefix, "suffix": msg.suffix}),
            ));
        }
        out
    }
}

/// The door whose receipt a write waits for.
fn door_of(request: &Request) -> Option<&'static str> {
    match request {
        Request::SetCwd(_) => Some("cwd"),
        Request::Rename(_) => Some("name"),
        Request::SwitchModel(_) => Some("switch"),
        Request::SetDefaultModel(_) => Some("default_model"),
        Request::Compact { .. } => Some("compact"),
        _ => None,
    }
}
