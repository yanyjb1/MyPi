//! The words the front end and the api both use: a model address, a compaction budget.

/// A model address: the provider and the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelId {
    pub provider: String,
    pub model: String,
}

/// How much of the history a compaction keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// Characters of the tail.
    Chars(u64),
    /// Newest blocks.
    Blocks(u64),
}
