//! One slot of the finished conversation: a number, and what sits on it.

/// How loud one of this front end's own lines is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// How long a line of this front end's own stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Life {
    /// Dies when the next lasting thing lands.
    Short,
    /// Stays for the rest of the conversation.
    Long,
}

/// What one slot holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotKind {
    /// One block, named upstream by this id.
    Block(i64),
    /// A line this front end says. It stays here and never goes upstream.
    Notice {
        text: String,
        level: Level,
        life: Life,
    },
    /// A dead slot: it keeps its number, draws nothing, and takes no budget.
    Next,
}

/// One slot: a number, and what sits on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub num: i64,
    pub kind: SlotKind,
}
