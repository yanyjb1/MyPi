//! The write half: everything that lands goes through here. The **when** belongs to the inlet.

pub(crate) mod aliases;
pub(crate) mod artifacts;
pub(crate) mod batches;
pub(crate) mod compaction;
pub(crate) mod effects;
pub(crate) mod memos;
pub(crate) mod sessions;
pub(crate) mod shared;
