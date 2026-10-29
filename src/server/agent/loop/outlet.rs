use std::sync::mpsc::Sender;

use crate::server::agent::request::content::RetrySink;
use crate::server::agent::request::message::NoticeLevel;
use crate::server::agent::request::retry::RetryNotice;

/// A live line that never becomes a block.
#[derive(Debug, Clone)]
pub struct Event {
    pub session: i64,
    pub level: &'static str,
    pub content: String,
}

/// The loop's outlet: live signals leave as [`Event`]s.
pub struct EventOutlet {
    tx: Sender<Event>,
    session: i64,
}

impl EventOutlet {
    pub fn new(tx: Sender<Event>, session: i64) -> Self {
        Self { tx, session }
    }
}

impl Outlet for EventOutlet {
    fn tool_event(&mut self, _event: &ToolEvent) {}

    fn tool_output(&mut self, _call_id: &str, _text: &str) {}

    fn retry(&mut self, notice: &RetryNotice) {
        let level = match notice.level {
            NoticeLevel::Info => "info",
            NoticeLevel::Warn => "warn",
            NoticeLevel::Error => "error",
        };
        let _ = self.tx.send(Event {
            session: self.session,
            level,
            content: format!(
                "{} (attempt {}/{})",
                notice.reason, notice.attempt, notice.max
            ),
        });
    }

    fn budget_exhausted(&mut self) {
        let _ = self.tx.send(Event {
            session: self.session,
            level: "warn",
            content: "the tool budget is exhausted".to_string(),
        });
    }
}

/// Loop-to-front-end events.
pub trait Outlet {
    fn tool_event(&mut self, event: &ToolEvent);
    fn tool_output(&mut self, call_id: &str, text: &str);
    fn retry(&mut self, notice: &RetryNotice);
    fn budget_exhausted(&mut self);
}

/// One tool call's outward moments.
#[derive(Debug, Clone)]
pub enum ToolEvent {
    Start {
        call_id: String,
        name: String,
        args: String,
        intent: String,
        text: String,
        first: bool,
    },
    Progress {
        call_id: String,
        text: String,
    },
    Finish {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
        memo: Option<serde_json::Value>,
        duration_ms: u64,
    },
}

/// Request retry notices forwarded to [`Outlet`].
pub struct RetryToOutlet<'a> {
    out: &'a mut dyn Outlet,
}

impl<'a> RetryToOutlet<'a> {
    pub fn new(out: &'a mut dyn Outlet) -> Self {
        Self { out }
    }
}

impl RetrySink for RetryToOutlet<'_> {
    fn retry(&mut self, notice: &RetryNotice) {
        self.out.retry(notice);
    }
}
