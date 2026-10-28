use crate::server::agent::request::content::RetrySink;
use crate::server::agent::request::retry::RetryNotice;

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
