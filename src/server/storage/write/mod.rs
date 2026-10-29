//! The write half: one module per table. A row names its table and its fields; nothing here
//! decides what a caller meant.

pub(crate) mod aliases;
pub(crate) mod apply;
pub(crate) mod artifacts;
pub(crate) mod blocks;
pub(crate) mod memos;
pub(crate) mod sessions;
pub(crate) mod shared;
pub(crate) mod subsessions;
pub(crate) mod turns;
