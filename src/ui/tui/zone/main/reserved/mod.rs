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
use crate::ui::tui::zone::{Claim, Frame, Kind, MemoAsk, Request, TermSize, name};

use crate::ui::tui::zone::main::input::{Currency, Lang};

use hand::{Hand, IDLE_ROWS, MAX_ROWS, Report};
use pools::Pools;

/// The classes the desk reads.
const KINDS: &[Kind] = &[Kind::Call, Kind::Post];

/// The desk.
#[derive(Default)]
pub struct ReservedZone {
    completion: hands::completion::CompletionService,
    usage: hands::usage::UsagePanel,
    memo: hands::memo::MemoHand,
    /// Who is on duty now.
    on_duty: Option<Hire>,
    /// The report taken the first frame after arrival: height and keys frozen here.
    report: Option<Report>,
    /// Rows the desk allocated this frame.
    want: u16,
    /// A reply for the editor, waiting for the zone to take it.
    reply: Option<Reply>,
    /// The workspace: the launch directory until a conversation reports its own.
    cwd: String,
    /// Sentences waiting in the slot, as the input zone counts them.
    queued: usize,
    /// Frames this zone owes the session.
    outbox: Vec<Request>,
}

impl ReservedZone {
    /// Nothing is claimed up front; the desk only borrows.
    pub const CLAIMS: &'static [Claim] = &[];

    /// The hand for one hire signal. One `match`: a new hand touches here only.
    fn hand(&mut self, who: &Hire) -> &mut dyn Hand {
        match who {
            Hire::Completion => &mut self.completion,
            Hire::Usage => &mut self.usage,
            Hire::Memo(_) => &mut self.memo,
        }
    }

    /// Call a hand in. Whoever is on duty leaves first; the name pools are asked every time.
    pub fn hire(&mut self, who: Hire) {
        // A second memo ask lands in the hand on duty: its frozen height never moves.
        if let (Some(Hire::Memo(_)), Hire::Memo(ask)) = (self.on_duty.clone(), &who) {
            self.memo.ask(ask);
            if !matches!(ask, MemoAsk::List) {
                self.outbox.push(Request::Memo(MemoAsk::List));
            }
            self.outbox.push(Request::Memo(ask.clone()));
            return;
        }
        if self.on_duty == Some(who.clone()) {
            return;
        }
        if let Some(prev) = self.on_duty.clone() {
            self.hand(&prev).left();
        }
        self.on_duty = Some(who.clone());
        self.report = None;
        match &who {
            Hire::Completion => self.outbox.push(Request::Pools),
            Hire::Usage => {
                self.outbox.push(Request::Usage);
                self.outbox.push(Request::Config {
                    key: hands::usage::ALIGN_KEY.to_string(),
                });
            }
            // The list comes first: the height is read off it, the ask lands after.
            Hire::Memo(ask) => {
                self.memo.ask(ask);
                if !matches!(ask, MemoAsk::List) {
                    self.outbox.push(Request::Memo(MemoAsk::List));
                }
                self.outbox.push(Request::Memo(ask.clone()));
            }
        }
        self.hand(&who).arrived();
        let cwd = self.cwd.clone();
        self.hand(&who).place(&cwd);
    }

    /// The editor's text and cursor, after every edit. A hand that needs them reads them;
    /// the report is what the desk needs, not the other way round.
    pub fn text(&mut self, text: &str, cursor: usize) {
        if let Some(who) = self.on_duty.clone() {
            self.hand(&who).text(text, cursor);
        }
    }

    /// The cursor's column, after every draw.
    pub fn col(&mut self, col: u16) {
        if let Some(who) = self.on_duty.clone() {
            self.hand(&who).col(col);
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

    /// What the memo hand asked for, as requests.
    fn drain_memo(&mut self) {
        for out in self.memo.drain() {
            match out {
                hands::memo::Out::Ask(ask) => self.outbox.push(Request::Memo(ask)),
                hands::memo::Out::Copy(text) => self.outbox.push(Request::Copy(text)),
            }
        }
    }

    /// Frame start: drop a hand that left, take the report of a fresh one.
    pub(super) fn settle(&mut self) {
        let Some(who) = self.on_duty.clone() else {
            return;
        };
        if !self.hand(&who).stays() {
            self.hand(&who).left();
            self.on_duty = None;
            self.report = None;
            return;
        }
        if self.report.is_none() && self.hand(&who).ready() {
            self.report = Some(self.hand(&who).report());
        }
    }

    /// Idle: one blank row. On duty: the hand's own rows, padded to what it was given.
    pub fn render_rows(&mut self, term: TermSize, accent: &Accent) -> Vec<Line<'static>> {
        self.settle();
        let rows = usize::from(self.want.max(IDLE_ROWS));
        let Some(who) = self.on_duty.clone() else {
            return vec![self.idle_row(accent)];
        };
        let mut out = self.hand(&who).draw(term.cols, rows as u16, accent);
        out.resize(rows, Line::default());
        out
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
                if let Some(who) = self.on_duty.clone() {
                    self.hand(&who).pools(&pools);
                }
            }
            ("call", "usage") => {
                let asks = self.usage.take(&frame.content);
                self.outbox.extend(asks);
            }
            ("call", "memo") => {
                self.memo.read(&frame.content);
                self.drain_memo();
            }
            ("post", "memo") => {
                self.memo.wrote(&frame.content);
                self.drain_memo();
            }
            ("call", "config") => {
                if frame.content.get("key").and_then(Value::as_str) == Some(hands::usage::ALIGN_KEY)
                    && let Some(name) = frame.content.get("value").and_then(Value::as_str)
                    && let Some(align) = hands::usage::Align::parse(name)
                {
                    self.usage.set_align(align);
                }
            }
            ("stream", _) | ("local", _) => {
                if let Some(who) = self.on_duty.clone() {
                    self.hand(&who).on_event("stream", &frame.content);
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
        let Some(who) = self.on_duty.clone() else {
            return false;
        };
        if let Some(reply) = self.hand(&who).on_key(key) {
            self.reply = Some(reply);
        }
        self.drain_memo();
        true
    }

    fn attach(&mut self, _session: i64, _size: TermSize) {
        self.on_duty = None;
        self.report = None;
        self.reply = None;
        self.want = 0;
        self.memo = hands::memo::MemoHand::default();
    }

    fn detach(&mut self) {
        if let Some(who) = self.on_duty.clone() {
            self.hand(&who).left();
        }
        self.on_duty = None;
        self.report = None;
        self.reply = None;
        self.want = 0;
        self.memo = hands::memo::MemoHand::default();
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
                    if let Some(who) = self.on_duty.clone() {
                        let cwd = self.cwd.clone();
                        self.hand(&who).place(&cwd);
                    }
                }
            }
            name::LANG => {
                if let Some(code) = content.get("lang").and_then(Value::as_str)
                    && let Some(lang) = Lang::parse(code)
                {
                    self.usage.set_lang(lang);
                }
            }
            name::CURRENCY => {
                if let Some(code) = content.get("currency").and_then(Value::as_str)
                    && let Some(currency) = Currency::parse(code)
                {
                    self.usage.set_currency(currency);
                }
            }
            name::MODEL_ENTRY => {
                self.usage.display(content);
            }
            _ => {
                if let Some(who) = self.on_duty.clone() {
                    self.hand(&who).on_event(event, content);
                }
            }
        }
    }
}

impl Pane for ReservedZone {
    fn request_height(&self, _term: TermSize) -> u16 {
        match self.report {
            Some(report) => report.rows.min(MAX_ROWS),
            None => IDLE_ROWS,
        }
    }

    fn assign(&mut self, _term: TermSize, rows: u16) {
        self.want = rows.max(IDLE_ROWS);
    }
}
