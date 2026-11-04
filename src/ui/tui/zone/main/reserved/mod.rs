//! The desk: one temp hand at a time, its frozen report, its keys, its rows.

mod hand;
mod hands;
mod pools;

use ratatui::text::{Line, Span};
use serde_json::Value;

use super::desk::{Hire, Reply};
use super::pane::Pane;
use crate::ui::tui::action::Action;
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::SubZone;
use crate::ui::tui::zone::{Claim, Frame, Kind, Request, TermSize, name};

use hand::{Hand, IDLE_ROWS, Report};
use pools::Pools;

/// The classes the desk reads.
const KINDS: &[Kind] = &[Kind::Call];

/// The desk.
#[derive(Default)]
pub struct ReservedZone {
    completion: hands::completion::CompletionService,
    usage: hands::usage::UsagePanel,
    /// Who is on duty now.
    on_duty: Option<Hire>,
    /// The report taken the first frame after arrival: height and keys frozen here.
    report: Option<Report>,
    /// Rows the desk allocated this frame.
    want: u16,
    /// A reply for the editor, waiting for the zone to take it.
    reply: Option<Reply>,
    /// The workspace, as the conversation reports it.
    cwd: String,
    /// The name pools, once the session has answered.
    pools: Option<Pools>,
    /// Sentences waiting in the slot, as the input zone counts them.
    queued: usize,
    /// The pools were asked for; one ask per attachment.
    asked: bool,
    /// Frames this zone owes the session.
    outbox: Vec<Request>,
}

impl ReservedZone {
    /// Nothing is claimed up front; the desk only borrows.
    pub const CLAIMS: &'static [Claim] = &[];

    /// The hand for one hire signal. One `match`: a new hand touches here only.
    fn hand(&mut self, who: Hire) -> &mut dyn Hand {
        match who {
            Hire::Completion => &mut self.completion,
            Hire::Usage => &mut self.usage,
        }
    }

    /// Call a hand in. Whoever is on duty leaves first.
    pub fn hire(&mut self, who: Hire) {
        if let Some(prev) = self.on_duty
            && prev != who
        {
            self.hand(prev).left();
        }
        self.on_duty = Some(who);
        self.report = None;
        if who == Hire::Usage {
            self.outbox
                .push(Request::call("usage", serde_json::json!({})));
        }
        self.hand(who).arrived();
        let cwd = self.cwd.clone();
        self.hand(who).place(&cwd);
        if let Some(pools) = &self.pools {
            let pools = pools.clone();
            self.hand(who).pools(&pools);
        }
    }

    /// The editor's text and cursor, after every edit. A hand that needs them reads them;
    /// the report is what the desk needs, not the other way round.
    pub fn text(&mut self, text: &str, cursor: usize) {
        if let Some(who) = self.on_duty {
            self.hand(who).text(text, cursor);
        }
    }

    /// The cursor's column, after every draw.
    pub fn col(&mut self, col: u16) {
        if let Some(who) = self.on_duty {
            self.hand(who).col(col);
        }
    }

    /// The idle line: how many sentences wait, or one blank row.
    fn idle_row(&self, accent: &Accent) -> Line<'static> {
        if self.queued == 0 {
            return Line::from("");
        }
        let noun = if self.queued == 1 {
            "message"
        } else {
            "messages"
        };
        Line::from(vec![
            accent.span(format!("{} {noun} queued", self.queued)),
            Span::raw(", alt+up to edit."),
        ])
    }

    /// A reply the editor has to apply.
    pub fn take_reply(&mut self) -> Option<Reply> {
        self.reply.take()
    }

    /// Frames this zone asked the session for.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }

    /// Frame start: drop a hand that left, take the report of a fresh one.
    pub(super) fn settle(&mut self) {
        let Some(who) = self.on_duty else {
            return;
        };
        if !self.hand(who).stays() {
            self.hand(who).left();
            self.on_duty = None;
            self.report = None;
            return;
        }
        if self.report.is_none() && self.hand(who).ready() {
            self.report = Some(self.hand(who).report());
        }
    }

    /// Idle: one blank row. On duty: the hand draws its own rows.
    pub fn render_rows(&mut self, term: TermSize, accent: &Accent) -> Vec<Line<'static>> {
        self.settle();
        let rows = self.want.max(IDLE_ROWS);
        let Some(who) = self.on_duty else {
            return vec![self.idle_row(accent)];
        };
        self.hand(who).draw(term.cols, rows, accent)
    }
}

impl SubZone for ReservedZone {
    fn claims(&self) -> &'static [Claim] {
        Self::CLAIMS
    }

    fn kinds(&self) -> &'static [Kind] {
        KINDS
    }

    fn borrowed(&self) -> &'static [Claim] {
        self.report.map(|r| r.keys).unwrap_or(&[])
    }

    fn deliver_event(&mut self, frame: &Frame, _size: TermSize) -> Vec<Request> {
        match (frame.class.as_str(), frame.name.as_str()) {
            ("call", "pools") => {
                let pools = Pools::of(&frame.content);
                if let Some(who) = self.on_duty {
                    self.hand(who).pools(&pools);
                }
                self.pools = Some(pools);
            }
            ("call", "usage") => {
                self.usage.take(&frame.content);
            }
            ("stream", _) | ("local", _) => {
                if let Some(who) = self.on_duty {
                    self.hand(who).on_event("stream", &frame.content);
                }
            }
            _ => {}
        }
        self.take_requests()
    }

    fn deliver_action(&mut self, a: &Action) -> bool {
        self.settle();
        let Action::Key(key) = a else {
            return false;
        };
        let Some(who) = self.on_duty else {
            return false;
        };
        if let Some(reply) = self.hand(who).on_key(key) {
            self.reply = Some(reply);
        }
        true
    }

    fn attach(&mut self, _session: i64, _size: TermSize) {
        self.cwd.clear();
        self.pools = None;
        self.asked = false;
        self.on_duty = None;
        self.report = None;
        self.reply = None;
        self.want = 0;
        self.outbox
            .push(Request::call("pools", serde_json::json!({})));
        self.asked = true;
    }

    fn detach(&mut self) {
        if let Some(who) = self.on_duty {
            self.hand(who).left();
        }
        self.on_duty = None;
        self.report = None;
        self.reply = None;
        self.want = 0;
        self.cwd.clear();
        self.pools = None;
        self.asked = false;
    }

    fn hear(&mut self, event: &str, content: &Value) {
        match event {
            name::QUEUE => {
                self.queued = content
                    .get("count")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize;
            }
            name::CWD => {
                if let Some(path) = content.get("path").and_then(Value::as_str) {
                    self.cwd.clear();
                    self.cwd.push_str(path);
                    if let Some(who) = self.on_duty {
                        let cwd = self.cwd.clone();
                        self.hand(who).place(&cwd);
                    }
                }
            }
            _ => {
                if let Some(who) = self.on_duty {
                    self.hand(who).on_event(event, content);
                }
            }
        }
    }
}

impl Pane for ReservedZone {
    fn request_height(&self, _term: TermSize) -> u16 {
        self.report.map(|r| r.rows).unwrap_or(IDLE_ROWS)
    }

    fn assign(&mut self, _term: TermSize, rows: u16) {
        self.want = rows.max(IDLE_ROWS);
    }
}
