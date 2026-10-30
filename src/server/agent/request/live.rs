//! The live line: what a watching front end hears while a request runs.

use std::sync::mpsc::Sender;

/// One live moment, addressed to one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub session: i64,
    pub element: Element,
}

/// The three kinds that share one channel: the stream, a notice, a tool's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Element {
    Stream(Stream),
    Notice(Notice),
    Tool(ToolEvent),
}

/// One piece of the reply as it arrives, or the end of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stream {
    Text(String),
    Thinking(String),
    /// Tool arguments, fragment by fragment; the call id is the only label they need.
    Arguments {
        call_id: String,
        text: String,
    },
    /// One block ends here.
    Boundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

/// Something said to whoever is watching. Never stored, never part of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: String,
}

/// One tool call's live moments. The loop fills these.
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

/// The sending end, bound to one session.
#[derive(Clone)]
pub struct Line {
    tx: Sender<Event>,
    session: i64,
}

impl Line {
    pub fn new(tx: Sender<Event>, session: i64) -> Self {
        Self { tx, session }
    }

    pub fn send(&self, element: Element) {
        let _ = self.tx.send(Event {
            session: self.session,
            element,
        });
    }

    pub fn stream(&self, piece: Stream) {
        self.send(Element::Stream(piece));
    }

    pub fn notice(&self, level: NoticeLevel, text: impl Into<String>) {
        self.send(Element::Notice(Notice {
            level,
            text: text.into(),
        }));
    }
}
