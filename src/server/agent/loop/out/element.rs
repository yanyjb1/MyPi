//! What one live line carries.

/// Which part of the model output an increment belongs to. Content alone
/// cannot tell thinking from text; tool arguments carry their call id instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Text,
    Thinking,
}

/// One tool call's live moments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolEvent {
    Start {
        call_id: String,
        name: String,
        args: String,
        intent: Option<String>,
        text: Option<String>,
    },
    Progress {
        call_id: String,
        text: String,
    },
    End {
        call_id: String,
        ok: bool,
        result: String,
        memo: Option<String>,
        duration_ms: u64,
    },
}

/// One live element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Element {
    Increment { part: Part, text: String },
    Boundary,
    Tool(ToolEvent),
    Notice { level: NoticeLevel, text: String },
}

/// The severity of a live notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

/// One element addressed to one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub session: i64,
    pub element: Element,
}
