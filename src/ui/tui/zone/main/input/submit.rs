//! Enter: a line leaves as a turn, a command, or a sentence in the mailbox.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::command::{self, Call, Done};
use super::notice::Notice;
use super::queue::Outgoing;
use super::route::{self, Route};
use super::zone::InputZone;
use crate::ui::tui::zone::Frame;
use crate::ui::tui::zone::main::input::command::CommandMeta;
use crate::ui::tui::zone::{Shout, name};

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
                && let Some(session) = self.session
            {
                self.shared
                    .mail
                    .send(Frame::new("post", "interrupt", json!({"session": session})));
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
                let prefix = self.auto_prefix();
                self.enqueue(prefix, text);
                true
            }
            Route::Message => {
                let prefix = self.auto_prefix();
                self.send(prefix, text);
                true
            }
        }
    }

    /// One command line: run it, then take the box away unless it asks to stay.
    fn run(&mut self, meta: &'static CommandMeta, args: &str) {
        let call = Call {
            args,
            session: self.session,
            cwd: &self.data.cwd,
            efforts: &self.data.efforts,
            draft: &self.draft,
        };
        match (meta.run)(&call) {
            Done::Write(write) => self.wrote(write),
            Done::Submit { prefix, text } => {
                self.leave(prefix, text);
                return;
            }
            Done::Exit(exit) => self.pending_exits.push(exit),
            Done::Hire(who) => self.pending_hire = Some(who),
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

    /// One write leaves: what settles here settles now; the two doors the server must settle wait for their receipt.
    fn wrote(&mut self, write: command::Write) {
        match &write.effect {
            Some(super::super::desk::Effect::Rename(name)) => {
                self.data.name = name.clone();
                // A session's own name repaints the whole front end; a draft has none to repaint.
                if self.session.is_some() {
                    self.shared.palette.shift();
                }
            }
            Some(super::super::desk::Effect::Switch(address)) => {
                self.model(address);
            }
            Some(super::super::desk::Effect::Effort(effort)) => self.data.effort = effort.clone(),
            None => {}
        }
        self.shared.mail.send(write.letter.clone());
        match door_of(&write.letter.name) {
            Some(door) => self.pending_notice.push_back(Waiting {
                door,
                notice: write.notice,
            }),
            None => self.say(&write.notice),
        }
        self.bump();
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
    pub(super) fn receipt(&mut self, door: &str, content: &Value) {
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
            // The write did not land, and the user is owed the reason: a refusal spoken nowhere is a silence.
            self.say_refused(content);
            return;
        }
        if door == "compact" {
            self.shouts.push(Shout::new(name::COMPACT_START, json!({})));
        }
        if let Notice::Default(id) = &wait.notice
            && self.session.is_none()
        {
            self.model(id);
        }
        self.say(&wait.notice);
    }

    /// A sentence sent while a round runs: the slot takes it, the round is untouched.
    fn enqueue(&mut self, prefix: Option<String>, text: String) {
        self.take_line();
        let Some(session) = self.session else {
            return;
        };
        if let Some(letter) = self.queue.push(Outgoing::new(prefix, text), session) {
            self.shared.mail.send(letter);
        }
        self.shouts
            .push(Shout::new(name::QUEUE, json!({"count": self.queue.len()})));
        self.enter_armed = Some(Instant::now());
    }

    /// A sentence sent while nothing runs: it opens a turn.
    fn send(&mut self, prefix: Option<String>, text: String) {
        self.take_line();
        self.in_flight = Some(text.clone());
        // The server never echoes a writer's own letter: history draws it from here, at once.
        self.shouts
            .push(Shout::new(name::ECHO, json!({"text": text})));
        let Some(session) = self.session else {
            // The row does not exist yet: it is opened with this text, which follows it.
            self.pending_open = Some((prefix, text));
            self.opening_sent = true;
            let letter = self.draft.opening();
            self.shared.mail.send(letter);
            return;
        };
        self.shared.mail.send(Frame::new(
            "subscribe",
            "submit",
            json!({"session": session, "prefix": prefix, "text": text}),
        ));
        self.data.busy = true;
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": true})));
    }

    /// One message a command built: the mailbox while a round runs, a new turn otherwise.
    fn leave(&mut self, prefix: Option<String>, text: String) {
        self.prepare_pending = false;
        match self.data.busy {
            true => self.enqueue(prefix, text),
            false => self.send(prefix, text),
        }
    }

    /// The preparation a message carries on its own, when it is due.
    fn auto_prefix(&mut self) -> Option<String> {
        if !self.data.auto_prepare || !self.prepare_pending {
            return None;
        }
        self.prepare_pending = false;
        command::payload(&self.data.cwd, true)
    }

    /// The reply to a submit that opened nothing: the sentence comes back, and says why.
    pub(super) fn refused(&mut self, content: &Value) {
        let reason = crate::ui::lang::refused(self.data.lang, content)
            .unwrap_or_else(|| "the session is still in a turn".to_string());
        self.shouts
            .push(Shout::new(name::RETRACT, json!({"reason": reason})));
        if let Some(text) = self.in_flight.take() {
            self.prefill(&text);
        }
    }

    /// A write the server refused: one line, and nothing to retract.
    pub(super) fn say_refused(&mut self, content: &Value) {
        let reason = crate::ui::lang::refused(self.data.lang, content)
            .unwrap_or_else(|| "the write was refused".to_string());
        self.say(&Notice::Refused(reason));
    }

    /// The server took the slot: the head left the queue, and the next one takes the slot.
    pub(super) fn opened(&mut self) {
        // A turn just opened: the box is busy before its first piece arrives.
        self.data.busy = true;
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": true})));
        let Some(session) = self.session else {
            return;
        };
        let (taken, letter) = self.queue.took(session);
        if let Some(letter) = letter {
            self.shared.mail.send(letter);
        }
        self.shouts
            .push(Shout::new(name::QUEUE, json!({"count": self.queue.len()})));
        if let Some(msg) = taken {
            self.shouts.push(Shout::new(
                name::ECHO,
                json!({"text": msg.text, "prefix": msg.prefix, "suffix": msg.suffix}),
            ));
        }
    }
}

/// The door whose receipt a write waits for.
fn door_of(name: &str) -> Option<&'static str> {
    match name {
        "default_model" => Some("default_model"),
        "compact" => Some("compact"),
        _ => None,
    }
}
