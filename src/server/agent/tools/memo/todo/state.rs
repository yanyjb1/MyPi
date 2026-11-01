//! The list's shape: one entry, and how many are done.

use serde::{Deserialize, Serialize};

/// One line of the model's checklist. Its number is its position, 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

/// Count them: `(done, total)`.
pub fn counts(items: &[TodoItem]) -> (usize, usize) {
    (items.iter().filter(|item| item.done).count(), items.len())
}
