// The desk: one temp hand at a time, its frozen report, its keys, its rows.

mod hand;
mod hands;
mod pools;

use ratatui::text::{Line, Span};
use serde_json::{Value, json};

use super::desk::{Hire, MemoAsk, Reply};
use super::pane::Pane;
use crate::ui::tui::action::Action;
use crate::ui::tui::lang::Lang;
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::{Claim, Frame, Kind, Local, Shared, TermSize, name};

use crate::ui::tui::zone::main::input::Currency;

use hand::{Hand, IDLE_ROWS, MAX_ROWS, Report};
use pools::Pools;

// The classes the desk reads.
const KINDS: &[Kind] = &[Kind::Call, Kind::Post];

// The desk.
#[derive(Default)]
pub struct ReservedZone {
    completion: hands::completion::CompletionService,
    usage: hands::usage::UsagePanel,
    memo: hands::memo::MemoHand,

    // The post box, the palette and the draft every zone shares.
    shared: Shared,

    // The conversation this zone follows; `None` = a draft.
    session: Option<i64>,

    // Who is on duty now.
    on_duty: Option<Hire>,

    // The report taken the first frame after arrival: height and keys frozen here.
    report: Option<Report>,

    // Rows the desk allocated this frame.
    want: u16,

    // A reply for the editor, waiting for the zone to take it.
    reply: Option<Reply>,

    // The workspace: the launch directory until a conversation reports its own.
    cwd: String,

    // Sentences waiting in the slot, as the input zone counts them.
    queued: usize,

    // What the desk wants the app itself to do.
    locals: Vec<Local>,

    // Bumped whenever this zone's rows would differ.
    rev: u64,
}

impl ReservedZone {
    fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    // One memo ask, as the two doors spell it.
    fn memo_ask(&self, ask: &MemoAsk) {
        let scope = ask.scope().as_str();
        match ask {
            MemoAsk::List => {
                self.shared.mail.send(Frame::new(
                    "call",
                    "memo",
                    json!({"session": self.session, "op": "list"}),
                ));
            }
            MemoAsk::History { name, .. } => {
                let content =
                    json!({"session": self.session, "op": "history", "scope": scope, "name": name});
                self.shared.mail.send(Frame::new("call", "memo", content));
            }
            MemoAsk::Add { name, content, .. } => {
                let body = json!({"session": self.session, "op": "add", "scope": scope, "name": name, "content": content});
                self.shared.mail.send(Frame::new("post", "memo", body));
            }
            MemoAsk::Mod { name, content, .. } => {
                let body = json!({"session": self.session, "op": "mod", "scope": scope, "name": name, "content": content});
                self.shared.mail.send(Frame::new("post", "memo", body));
            }
            MemoAsk::Del { name, .. } => {
                let body =
                    json!({"session": self.session, "op": "del", "scope": scope, "name": name});
                self.shared.mail.send(Frame::new("post", "memo", body));
            }
        }
    }

    // One key, when a hand is on duty. A hand that answers hands its reply to the editor.
    fn act(&mut self, a: &Action) -> bool {
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

    // The hand for one hire signal. One `match`: a new hand touches here only.
    fn hand(&mut self, who: &Hire) -> &mut dyn Hand {
        match who {
            Hire::Completion => &mut self.completion,
            Hire::Usage => &mut self.usage,
            Hire::Memo(_) => &mut self.memo,
        }
    }

    // Call a hand in. Whoever is on duty leaves first; the name pools are asked every time.
    pub fn hire(&mut self, who: Hire) {
        // A second memo ask lands in the hand on duty: its frozen height never moves.
        if let (Some(Hire::Memo(_)), Hire::Memo(ask)) = (self.on_duty.clone(), &who) {
            self.memo.ask(ask);
            if !matches!(ask, MemoAsk::List) {
                self.memo_ask(&MemoAsk::List);
            }
            self.memo_ask(ask);
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
            Hire::Completion => {
                self.shared.mail.send(Frame::new(
                    "call",
                    "pools",
                    json!({"session": self.session, "cwd": self.cwd}),
                ));
            }
            Hire::Usage => {
                if let Some(session) = self.session {
                    self.shared
                        .mail
                        .send(Frame::new("call", "usage", json!({"session": session})));
                }
                self.shared.mail.send(Frame::new(
                    "call",
                    "config",
                    json!({"key": hands::usage::ALIGN_KEY}),
                ));
            }
            // The list comes first: the height is read off it, the ask lands after.
            Hire::Memo(ask) => {
                self.memo.ask(ask);
                if !matches!(ask, MemoAsk::List) {
                    self.memo_ask(&MemoAsk::List);
                }
                self.memo_ask(ask);
            }
        }
        self.hand(&who).arrived();
        let cwd = self.cwd.clone();
        self.hand(&who).place(&cwd);
        self.bump();
    }

    // The editor's text and cursor, after every edit; a hand that needs them reads them, and the report is what the desk needs, not the other way round.
    pub fn text(&mut self, text: &str, cursor: usize) {
        if let Some(who) = self.on_duty.clone() {
            self.hand(&who).text(text, cursor);
        }
    }

    // The cursor's column, after every draw.
    pub fn col(&mut self, col: u16) {
        if let Some(who) = self.on_duty.clone() {
            self.hand(&who).col(col);
        }
    }

    // The idle line: how many sentences wait, or one blank row.
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

    // A reply the editor has to apply.
    pub fn take_reply(&mut self) -> Option<Reply> {
        self.reply.take()
    }

    // The asks this zone owes the front end itself.
    pub fn take_locals(&mut self) -> Vec<Local> {
        std::mem::take(&mut self.locals)
    }

    // What the memo hand asked for.
    fn drain_memo(&mut self) {
        for out in self.memo.drain() {
            match out {
                hands::memo::Out::Ask(ask) => self.memo_ask(&ask),
                hands::memo::Out::Copy(text) => self.locals.push(Local::Copy(text)),
            }
        }
    }

    // Frame start: drop a hand that left, take the report of a fresh one.
    pub(super) fn settle(&mut self) {
        let before = (self.on_duty.clone(), self.report.is_some());
        self.settle_inner();
        let after = (self.on_duty.clone(), self.report.is_some());
        if before != after {
            self.bump();
        }
    }

    fn settle_inner(&mut self) {
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

    // Idle: one blank row. On duty: the hand's own rows, padded to what it was given.
    pub fn render_rows(&mut self, term: TermSize, accent: &Accent) -> Vec<Line<'static>> {
        let rows = usize::from(self.want.max(IDLE_ROWS));
        let Some(who) = self.on_duty.clone() else {
            return vec![self.idle_row(accent)];
        };
        let mut out = self.hand(&who).draw(term.cols, rows as u16, accent);
        out.resize(rows, Line::default());
        out
    }
}

impl ReservedZone {
    pub(in crate::ui::tui::zone::main) fn kinds(&self) -> &'static [Kind] {
        KINDS
    }

    pub(in crate::ui::tui::zone::main) fn revision(&self) -> u64 {
        self.rev
    }

    pub(in crate::ui::tui::zone::main) fn borrowed(&self) -> &'static [Claim] {
        self.report.map(|r| r.keys).unwrap_or(&[])
    }

    pub(in crate::ui::tui::zone::main) fn deliver_event(&mut self, frame: &Frame, _size: TermSize) {
        match (frame.kind.as_str(), frame.name.as_str()) {
            ("call", "pools") => {
                let pools = Pools::of(&frame.payload);
                if let Some(who) = self.on_duty.clone() {
                    self.hand(&who).pools(&pools);
                }
            }
            ("call", "usage") => {
                for key in self.usage.take(&frame.payload) {
                    self.shared
                        .mail
                        .send(Frame::new("call", "config", json!({"key": key})));
                }
            }
            ("call", "memo") => {
                self.memo.read(&frame.payload);
                self.drain_memo();
            }
            ("post", "memo") => {
                self.memo.wrote(&frame.payload);
                self.drain_memo();
            }
            ("call", "config") => {
                if frame.payload.get("key").and_then(Value::as_str) == Some(hands::usage::ALIGN_KEY)
                    && let Some(name) = frame.payload.get("value").and_then(Value::as_str)
                    && let Some(align) = hands::usage::Align::parse(name)
                {
                    self.usage.set_align(align);
                }
            }
            ("stream", _) | ("local", _) => {
                if let Some(who) = self.on_duty.clone() {
                    self.hand(&who).on_event("stream", &frame.payload);
                }
            }
            _ => {}
        }
        self.bump();
    }

    pub(in crate::ui::tui::zone::main) fn deliver_action(&mut self, a: &Action) -> bool {
        self.settle();
        let consumed = self.act(a);
        self.bump();
        consumed
    }

    pub(in crate::ui::tui::zone::main) fn attach(&mut self, session: i64, _size: TermSize) {
        self.session = Some(session);
        self.on_duty = None;
        self.report = None;
        self.reply = None;
        self.want = 0;
        self.memo = hands::memo::MemoHand::default();
        // The bill belongs to one session: a different one starts from nothing.
        self.usage.reset();
        self.bump();
    }

    pub(in crate::ui::tui::zone::main) fn detach(&mut self) {
        self.session = None;
        if let Some(who) = self.on_duty.clone() {
            self.hand(&who).left();
        }
        self.on_duty = None;
        self.report = None;
        self.reply = None;
        self.want = 0;
        self.memo = hands::memo::MemoHand::default();
        self.usage.reset();
        self.bump();
    }

    pub(in crate::ui::tui::zone::main) fn hear(&mut self, event: &str, content: &Value) {
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
                    self.completion.set_lang(lang);
                    self.memo.set_lang(lang);
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
                self.completion.set_model_entry(content);
            }
            _ => {
                if let Some(who) = self.on_duty.clone() {
                    self.hand(&who).on_event(event, content);
                }
            }
        }
        self.bump();
    }
}

impl ReservedZone {
    // The three shared handles arrive once, at boot.
    pub(super) fn bind(&mut self, shared: Shared) {
        self.shared = shared;
    }
}

impl Pane for ReservedZone {
    fn request_height(&mut self, _term: TermSize) -> u16 {
        match self.report {
            Some(report) => report.rows.min(MAX_ROWS),
            None => IDLE_ROWS,
        }
    }

    fn assign(&mut self, _term: TermSize, rows: u16) {
        let want = rows.max(IDLE_ROWS);
        if self.want != want {
            self.want = want;
            self.bump();
        }
    }
}
