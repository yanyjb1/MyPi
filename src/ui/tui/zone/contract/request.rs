//! What a zone asks the app to land on the session.

pub use crate::ui::session::{ModelId, Retention};

/// Where a memo hangs: one session's blocks, or every session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoScope {
    Current,
    Global,
}

impl MemoScope {
    /// The wire's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Global => "global",
        }
    }
}

/// One `/memo` ask: the panel's list, one name's versions, or a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoAsk {
    List,
    History {
        scope: MemoScope,
        name: String,
    },
    Add {
        scope: MemoScope,
        name: String,
        content: String,
    },
    Mod {
        scope: MemoScope,
        name: String,
        content: String,
    },
    Del {
        scope: MemoScope,
        name: String,
    },
}

/// One ask. The zone says what it wants; the app names the door.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    // Reads.
    Init,
    Meta,
    Usage,
    Pools,
    Config {
        key: String,
    },
    Blocks {
        ranges: Vec<[i64; 2]>,
    },
    Sessions {
        under: Option<String>,
    },
    Memo(MemoAsk),
    // Writes.
    Submit(String),
    Interrupt,
    Mailbox {
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    },
    ClearMailbox,
    Rename(Option<String>),
    SetCwd(String),
    SwitchModel(ModelId),
    SetDefaultModel(ModelId),
    Compact {
        retention: Option<Retention>,
        focus: Option<String>,
    },
    Delete(i64),
    Rewind {
        turn: i64,
        block: i64,
    },
    // The app's own.
    /// Text for the system clipboard; the loop writes it out.
    Copy(String),
    Attach(i64),
    Detach,
    Structure,
    Quit,
    /// The profile a session will open with; it lives in this front end until then.
    PickProfile(Option<String>),
}
