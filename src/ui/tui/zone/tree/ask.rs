//! The tree's range request. Landing belongs to the loop.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeAsk {
    Range { start: i64, end: i64 },
}
