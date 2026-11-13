//! The live line: what a watching front end hears while a request runs.

use std::sync::mpsc::Sender;

use super::message::Usage;

/// One live moment, addressed to one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub session: i64,
    pub element: Element,
}

/// The kinds that share one channel: the stream, a notice, a tool's state, a turn opening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Element {
    Stream(Stream),
    Notice(Notice),
    Tool(ToolEvent),
    /// The front end's slot became this turn; the api names the frames after it.
    Mailbox {
        turn: i64,
    },
    /// A turn closed: the id ranges its blocks occupy, in landing order, and whether it is the
    /// compaction that rewrote the history.
    Structure {
        turn: i64,
        ranges: Vec<[i64; 2]>,
        compact: bool,
    },
}

/// One piece of the reply as it arrives, or the end of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stream {
    Text {
        text: String,
        usage: Option<Usage>,
    },
    Thinking {
        text: String,
        usage: Option<Usage>,
    },
    /// Tool arguments, fragment by fragment; the name is empty until the provider states it.
    Arguments {
        call_id: String,
        name: String,
        text: String,
        usage: Option<Usage>,
    },
    /// One block ends here.
    Boundary,
    /// One wake message: its own block, so each reminder keeps its own text.
    Wake(String),
}

impl Stream {
    pub fn text(text: impl Into<String>) -> Stream {
        Stream::Text {
            text: text.into(),
            usage: None,
        }
    }

    pub fn thinking(text: impl Into<String>) -> Stream {
        Stream::Thinking {
            text: text.into(),
            usage: None,
        }
    }

    pub fn arguments(
        call_id: impl Into<String>,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> Stream {
        Stream::Arguments {
            call_id: call_id.into(),
            name: name.into(),
            text: text.into(),
            usage: None,
        }
    }
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
        truncated: Option<Truncation>,
    },
}

/// Size of the full text before the cut, not the part that stayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncation {
    pub lines: u64,
    pub bytes: u64,
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
