//! The desk's vocabulary: which temp hand sits at it, and what a hand asks of it.

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

impl MemoAsk {
    /// The scope every variant but `List` carries; a list covers both scopes at once.
    pub fn scope(&self) -> MemoScope {
        match self {
            Self::List => MemoScope::Global,
            Self::History { scope, .. }
            | Self::Add { scope, .. }
            | Self::Mod { scope, .. }
            | Self::Del { scope, .. } => *scope,
        }
    }
}

/// One temp hand. The desk seats at most one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hire {
    /// The completion popup.
    Completion,

    /// The `/usage` panel.
    Usage,

    /// The `/memo` panel, opened at one ask.
    Memo(MemoAsk),
}

/// What a hand wants the editor to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Replace the cursor's word with this text.
    Replace {
        from: usize,
        to: usize,
        text: String,
    },
}

/// What a write settles inside the input zone itself; the row is the server's business.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Rename(Option<String>),
    Switch(String),
    Effort(String),
}
