//! The list's shape: tasks, states, phases.

use serde::{Deserialize, Serialize};

/// One entry on the model's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoTask {
    pub content: String,
    pub status: TodoStatus,
    /// Why it is stuck (set by `block`); nothing otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

/// How far one task has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    /// Not started.
    Pending,
    /// In progress.
    InProgress,
    /// Done.
    Done,
    /// Stuck; `blocker` says why.
    Blocked,
    /// Dropped on purpose (not the same as not yet done).
    Abandoned,
}

/// A named group of tasks. Phases are what keep a long list readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoPhase {
    pub name: String,
    pub tasks: Vec<TodoTask>,
}

impl TodoPhase {
    /// How many in this phase are done.
    pub fn done(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.status == TodoStatus::Done)
            .count()
    }
}

/// Count them: `(done, total)`.
pub fn counts(phases: &[TodoPhase]) -> (usize, usize) {
    let total = phases.iter().map(|p| p.tasks.len()).sum();
    let done = phases.iter().map(|p| p.done()).sum();
    (done, total)
}
