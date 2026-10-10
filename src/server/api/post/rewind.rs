//! Rewind: cutting a session's list back to a block. The cut itself is storage's; the fork mode is
//! still a `todo!`.

use anyhow::Result;

use crate::server::storage::{Cut, Target, Writer};

/// Cut the session's list back to the picked block. The caller opens the next
/// round on what survived.
pub fn rewind(w: &Writer, t: Target) -> Result<Cut> {
    w.rewind(t)
}

/// The fork mode: leave the list alone and open a new session whose prefix is
/// the kept stretch. Not written yet.
pub fn rewind_non_prune(_w: &Writer, _t: Target) -> Result<Cut> {
    todo!("the fork mode: not moved yet")
}
