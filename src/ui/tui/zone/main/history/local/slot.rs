//! One slot of the finished conversation: a number, and what sits on it.

use super::super::product::Product;
use super::content::Block;

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

/// One block's content and the product derived from it at the current width.
pub struct Held {
    pub block: Block,
    pub product: Option<Product>,
    /// Admission order: the oldest content leaves first.
    pub at: u64,
}

/// What one slot holds.
pub enum SlotKind {
    /// One block, named upstream by this id; `None` = its content is not in hand.
    Block { id: i64, held: Option<Box<Held>> },
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
pub struct Slot {
    pub num: i64,
    pub kind: SlotKind,
}
