//! One tree row.

use super::layout::RowGeom;

/// One line of the tree. A block is a line.
pub struct Row {
    pub geom: RowGeom,
    pub block_id: i64,
    /// The round that was showing it: a rewind names both.
    pub turn: i64,
    /// Display text: `user: the first few words…`.
    pub text: String,
    /// The block's own text when it is a user message.
    pub body: Option<String>,
    /// Which fork it belongs to. Unused while only prune mode exists.
    pub fork: i64,
    /// Role class for colouring.
    pub kind: String,
}
