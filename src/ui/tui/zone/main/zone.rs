//! The chat zone: the host of history, input and reserved.

use serde_json::{Value, json};

use super::keys::Seat;
use super::{arbitrate, dispatch, history, input, keys, reserved};
use crate::ui::tui::zone::{
    Claim, Frame, Kind, Memo, Outcome, Request, Shout, SubZone, TermSize, Turns, Zone, name,
};

/// The fallback size before the first resize arrives.
const GUESS: TermSize = TermSize { cols: 80, rows: 24 };

/// Every class the three sub-zones consume between them.
const KINDS: &[Kind] = &[
    Kind::Call,
    Kind::Post,
    Kind::Subscribe,
    Kind::Stream,
    Kind::Notice,
];

#[derive(Default)]
pub struct MainZone {
    pub(super) history: history::zone::HistoryZone,
    pub(super) input: input::InputZone,
    pub(super) reserved: reserved::ReservedZone,
    pub(super) keys: keys::Borrowed,
    pub(super) size: Option<TermSize>,
    pub(super) heights: super::arbitrate::Heights,
    /// The color the input zone and the running cards wear.
    pub(super) accent: crate::ui::tui::theme::Accent,
    /// Events this zone itself put out, not yet on the bus.
    shouts: Vec<Shout>,
}

impl MainZone {
    /// Where the hardware cursor goes, in terminal coordinates.
    fn cursor_position(&self) -> Option<(u16, u16)> {
        super::arbitrate::cursor_position(self)
    }

    fn term(&self) -> TermSize {
        self.size.unwrap_or(GUESS)
    }

    /// Put one event of this zone's own on the bus.
    fn shout(&mut self, name: &'static str, content: Value) {
        self.shouts.push(Shout::new(name, content));
    }

    /// Hand every pending event to whoever did not put it out, then drain what that
    /// provoked. A shout during delivery lands on the same queue, never in a nested call.
    fn flush(&mut self) {
        loop {
            let own = std::mem::take(&mut self.shouts);
            let from_history = self.history.take_shouts();
            let from_input = self.input.take_shouts();
            let from_reserved = self.reserved.take_shouts();
            if own.is_empty()
                && from_history.is_empty()
                && from_input.is_empty()
                && from_reserved.is_empty()
            {
                return;
            }
            for event in own {
                self.history.hear(event.name, &event.content);
                self.input.hear(event.name, &event.content);
                self.reserved.hear(event.name, &event.content);
            }
            for event in from_history {
                self.input.hear(event.name, &event.content);
                self.reserved.hear(event.name, &event.content);
            }
            for event in from_input {
                self.history.hear(event.name, &event.content);
                self.reserved.hear(event.name, &event.content);
            }
            for event in from_reserved {
                self.history.hear(event.name, &event.content);
                self.input.hear(event.name, &event.content);
            }
        }
    }

    /// The conversation's directory, out of the rows that carry it.
    fn read_cwd(&mut self, frame: &Frame) {
        let row = match (frame.class.as_str(), frame.name.as_str()) {
            ("call", "meta") => &frame.content,
            ("stream", "change") => match frame.content.get("meta") {
                Some(row) => row,
                None => return,
            },
            _ => return,
        };
        let Some(path) = row.get("cwd").and_then(Value::as_str) else {
            return;
        };
        if self.input.knows_cwd(path) {
            return;
        }
        self.shout(name::CWD, json!({"path": path}));
    }
}

impl Zone for MainZone {
    fn keys(&self) -> Vec<Claim> {
        let mut out = Vec::new();
        out.extend_from_slice(history::zone::CLAIMS);
        out.extend_from_slice(input::InputZone::CLAIMS);
        out.extend_from_slice(reserved::ReservedZone::CLAIMS);
        out
    }

    fn kinds(&self) -> &'static [Kind] {
        KINDS
    }

    fn start(&mut self, size: TermSize, memo: &Memo) -> Vec<Request> {
        self.size = Some(size);
        if let Some(focus) = memo.read("tree.focus") {
            memo.delete("tree.focus");
            if let Some((turn, block)) = focus.split_once(':')
                && let (Ok(turn), Ok(block)) = (turn.parse::<i64>(), block.parse::<i64>())
            {
                self.history.focus(turn, block);
            }
        }
        self.flush();
        vec![Request::Structure]
    }

    fn session(&mut self, id: Option<i64>) -> Vec<Request> {
        let size = self.term();
        match id {
            Some(id) => {
                self.history.attach(id, size);
                self.input.attach(id, size);
                self.reserved.attach(id, size);
                self.flush();
                let mut requests = vec![
                    Request::call("init", json!({})),
                    Request::call("meta", json!({})),
                    Request::Structure,
                ];
                requests.extend(self.history.take_requests());
                requests.extend(self.reserved.take_requests());
                requests.extend(self.input.take_requests());
                requests
            }
            None => {
                self.history.detach();
                self.input.detach();
                self.reserved.detach();
                self.keys.clear();
                self.flush();
                Vec::new()
            }
        }
    }

    fn event(&mut self, frame: &Frame) -> Vec<Request> {
        if frame.name == "resized"
            && let Some(size) = TermSize::of(&frame.content)
        {
            self.size = Some(size);
        }
        if frame.is_local() && frame.name == "disconnected" {
            return vec![Request::Quit];
        }
        if frame.is_local() && frame.name == "accent" {
            if let Some(accent) = frame
                .content
                .get("color")
                .and_then(Value::as_str)
                .and_then(crate::ui::tui::theme::Accent::parse)
            {
                self.accent = accent;
            }
            return Vec::new();
        }
        self.read_cwd(frame);
        let kind = Kind::of(&frame.class, &frame.name);
        let size = self.term();
        let mut requests = Vec::new();
        let local = frame.is_local();
        if kind.is_some_and(|kind| self.history.kinds().contains(&kind)) || local {
            requests.extend(self.history.deliver_event(frame, size));
        }
        if kind.is_some_and(|kind| self.input.kinds().contains(&kind)) || local {
            requests.extend(self.input.deliver_event(frame, size));
        }
        if kind.is_some_and(|kind| self.reserved.kinds().contains(&kind)) || local {
            requests.extend(self.reserved.deliver_event(frame, size));
        }
        self.deliver_hires();
        self.flush();
        requests.extend(self.history.take_requests());
        requests.extend(self.reserved.take_requests());
        requests.extend(self.input.take_requests());
        requests
    }

    fn action(&mut self, action: &crate::ui::tui::action::Action) -> (Outcome, Vec<Request>) {
        let (outcome, mut requests) = dispatch::dispatch(self, action);
        requests.extend(self.history.take_requests());
        requests.extend(self.reserved.take_requests());
        requests.extend(self.input.take_requests());
        self.flush();
        (outcome, requests)
    }

    fn structure(&mut self, snapshot: &Turns) -> Vec<Request> {
        self.history.structure(snapshot);
        self.input.structure(snapshot);
        self.reserved.structure(snapshot);
        self.flush();
        Vec::new()
    }

    fn render(&mut self) -> Vec<ratatui::text::Line<'static>> {
        self.settle();
        let accent = self.accent;
        let width = self.term().cols;
        let mut out = self.history.render_rows(width, accent);
        for _ in 0..self.heights.gap {
            out.push(ratatui::text::Line::default());
        }
        out.extend(self.input.render_rows(self.term(), &accent));
        self.reserved.col(self.input.cursor_col);
        out.extend(self.reserved.render_rows(self.term(), &accent));
        out
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.cursor_position()
    }
}

impl MainZone {
    fn settle(&mut self) {
        arbitrate::settle(self);
        self.deliver_hires();
        self.flush();
    }

    /// A hand on duty holds the keys; a desk with nobody at it returns them.
    pub(super) fn sync_borrowed(&mut self) {
        let claims = self.reserved.borrowed();
        if claims.is_empty() {
            self.keys.release(Seat::Reserved);
        } else {
            self.keys.lend(claims, Seat::Reserved);
        }
    }

    /// The editor asked for a hand: seat it, and feed it the text it missed.
    fn deliver_hires(&mut self) {
        if let Some(who) = self.input.take_hire() {
            self.reserved.hire(who);
            self.sync_borrowed();
            let (text, cursor) = self.input.draft();
            self.reserved.text(&text, cursor);
        }
        if let Some(reply) = self.reserved.take_reply() {
            self.input.apply_reply(reply);
        }
        let (text, cursor) = self.input.draft();
        self.reserved.text(&text, cursor);
    }
}
