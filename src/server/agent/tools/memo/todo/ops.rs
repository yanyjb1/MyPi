//! The nine ops: how each is read, checked and applied; plus the two replies.

use anyhow::{Result, anyhow};

use super::state::{TodoPhase, TodoStatus, TodoTask, counts};
use crate::server::agent::tools::spec::Args;

// Positions in the parameter table; `TOOL` writes it in this order.
pub(super) const OP: usize = 0;
pub(super) const LIST: usize = 1;
pub(super) const ITEMS: usize = 2;
pub(super) const PHASE: usize = 3;
pub(super) const TASK: usize = 4;
pub(super) const REASON: usize = 5;

/// One phase's shape in `list`.
pub(super) const PHASE_RECORD: usize = 0;
pub(super) const PHASE_ITEMS: usize = 1;

/// One op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Replace the whole list (`list` = phases, or flat `items`).
    Init,
    /// Add entries to a phase (creating it when absent).
    Append,
    /// Mark one as in progress.
    Start,
    /// Mark one as done.
    Done,
    /// Give up on one (not the same as not yet done).
    Drop,
    /// One is stuck; `reason` says why.
    Block,
    /// Release a stuck one (back to not started).
    Unblock,
    /// Drop one.
    Rm,
    /// Report the current list, change nothing.
    View,
}

impl Op {
    /// The allowed values, for the manual.
    pub const ALL: &'static [&'static str] = &[
        "init", "append", "start", "done", "drop", "block", "unblock", "rm", "view",
    ];

    pub fn parse(s: &str) -> Option<Op> {
        Some(match s {
            "init" => Op::Init,
            "append" => Op::Append,
            "start" => Op::Start,
            "done" => Op::Done,
            "drop" => Op::Drop,
            "block" => Op::Block,
            "unblock" => Op::Unblock,
            "rm" => Op::Rm,
            "view" => Op::View,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Op::Init => "init",
            Op::Append => "append",
            Op::Start => "start",
            Op::Done => "done",
            Op::Drop => "drop",
            Op::Block => "block",
            Op::Unblock => "unblock",
            Op::Rm => "rm",
            Op::View => "view",
        }
    }

    /// Look, change nothing.
    pub fn is_read_only(self) -> bool {
        matches!(self, Op::View)
    }
}

/// One call, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub op: Op,
    /// `init`: the whole phased table.
    pub list: Vec<(String, Vec<String>)>,
    /// `init` (flat) and `append`: the entry texts.
    pub items: Vec<String>,
    /// `append`: which phase.
    pub phase: Option<String>,
    /// The entry this op names, copied **verbatim**.
    pub task: Option<String>,
    /// `block`: why.
    pub reason: Option<String>,
}

impl Call {
    /// Read from the parameter table, then shape-checked.
    pub fn read(args: &Args) -> Result<Call> {
        let name = args.choice(OP)?;
        let op = Op::parse(&name)
            .ok_or_else(|| anyhow!("unknown op `{name}` ({})", Op::ALL.join("/")))?;

        let mut list = Vec::new();
        for row in args.table(LIST)? {
            let phase = row.str(PHASE_RECORD)?;
            let items = row.str_list(PHASE_ITEMS)?;
            list.push((phase, items));
        }
        let call = Call {
            op,
            list,
            items: args.str_list(ITEMS)?,
            phase: args.opt_str(PHASE)?,
            task: args.opt_str(TASK)?.filter(|s| !s.trim().is_empty()),
            reason: args.opt_str(REASON)?,
        };
        call.validate()?;
        Ok(call)
    }

    /// Check the shape per op — errors the model can act on.
    fn validate(&self) -> Result<()> {
        match self.op {
            Op::Init => {
                anyhow::ensure!(
                    !self.list.is_empty() || !self.items.is_empty(),
                    "init needs `list: [{{phase, items}}]` or `items: [..]`"
                );
                anyhow::ensure!(
                    self.list.iter().all(|(_, i)| !i.is_empty()),
                    "every phase in `init` needs at least one item"
                );
            }
            Op::Append => {
                anyhow::ensure!(!self.items.is_empty(), "append needs `items: [..]`");
            }
            Op::Start | Op::Done | Op::Drop | Op::Block | Op::Unblock | Op::Rm => {
                anyhow::ensure!(
                    self.task.is_some(),
                    "{} needs `task`: the item's exact text, copied verbatim",
                    self.op.as_str()
                );
            }
            Op::View => {}
        }
        Ok(())
    }
}

/// Apply one call; returns the new list. Errors are **collected**, not fatal: the caller drops the
/// whole batch and every problem comes back in one pass.
pub fn apply(phases: &[TodoPhase], call: &Call) -> (Vec<TodoPhase>, Vec<String>) {
    let mut next = phases.to_vec();
    let mut errors = Vec::new();
    match call.op {
        Op::View => {}
        Op::Init => {
            next = if !call.list.is_empty() {
                call.list
                    .iter()
                    .map(|(name, items)| TodoPhase {
                        name: name.clone(),
                        tasks: items
                            .iter()
                            .map(|c| TodoTask {
                                content: c.clone(),
                                status: TodoStatus::Pending,
                                blocker: None,
                            })
                            .collect(),
                    })
                    .collect()
            } else {
                vec![TodoPhase {
                    name: "Tasks".to_string(),
                    tasks: call
                        .items
                        .iter()
                        .map(|c| TodoTask {
                            content: c.clone(),
                            status: TodoStatus::Pending,
                            blocker: None,
                        })
                        .collect(),
                }]
            };
        }
        Op::Append => {
            let phase = call.phase.clone().unwrap_or_else(|| "Tasks".to_string());
            let target = match next.iter_mut().find(|p| p.name == phase) {
                Some(p) => p,
                None => {
                    next.push(TodoPhase {
                        name: phase.clone(),
                        tasks: Vec::new(),
                    });
                    next.last_mut().expect("just pushed")
                }
            };
            for content in &call.items {
                if target.tasks.iter().any(|t| &t.content == content) {
                    errors.push(format!("{content:?} is already in the list"));
                    continue;
                }
                target.tasks.push(TodoTask {
                    content: content.clone(),
                    status: TodoStatus::Pending,
                    blocker: None,
                });
            }
        }
        Op::Rm => {
            let task = call.task.as_deref().unwrap_or_default();
            let mut removed = false;
            for phase in next.iter_mut() {
                let before = phase.tasks.len();
                phase.tasks.retain(|t| t.content != task);
                removed |= phase.tasks.len() != before;
            }
            if !removed {
                errors.push(format!("no item named {task:?} in the list"));
            }
            next.retain(|p| !p.tasks.is_empty());
        }
        // Every state-changing op looks up through here.
        Op::Start | Op::Done | Op::Drop | Op::Block | Op::Unblock => {
            let task = call.task.as_deref().unwrap_or_default();
            let mut found = false;
            for phase in next.iter_mut() {
                for t in phase.tasks.iter_mut() {
                    if t.content != task {
                        continue;
                    }
                    found = true;
                    match call.op {
                        Op::Start => {
                            t.status = TodoStatus::InProgress;
                            t.blocker = None;
                        }
                        Op::Done => {
                            t.status = TodoStatus::Done;
                            t.blocker = None;
                        }
                        Op::Drop => {
                            t.status = TodoStatus::Abandoned;
                            t.blocker = None;
                        }
                        Op::Block => {
                            t.status = TodoStatus::Blocked;
                            t.blocker = Some(
                                call.reason
                                    .clone()
                                    .unwrap_or_else(|| "(no reason given)".to_string()),
                            );
                        }
                        Op::Unblock => {
                            t.status = TodoStatus::Pending;
                            t.blocker = None;
                        }
                        _ => unreachable!("handled above"),
                    }
                }
            }
            if !found {
                errors.push(format!(
                    "no item named {task:?} — `task` MUST match the text you wrote, verbatim (use view to read the current list)"
                ));
            }
            // Only one item is in progress at a time; starting one demotes the previous.
            if call.op == Op::Start && found {
                for phase in next.iter_mut() {
                    for t in phase.tasks.iter_mut() {
                        if t.content != task && t.status == TodoStatus::InProgress {
                            t.status = TodoStatus::Pending;
                        }
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        // Dropped whole — see the module doc.
        return (phases.to_vec(), errors);
    }
    (next, errors)
}

/// The **view** reply: the whole list as text. An empty list gets a placeholder, not an empty
/// string.
pub fn rendered(phases: &[TodoPhase]) -> String {
    if phases.is_empty() {
        return "(the list is empty)".to_string();
    }
    let (done, total) = counts(phases);
    let mut out = String::new();
    for phase in phases {
        out.push_str(&format!("{}\n", phase.name));
        for t in &phase.tasks {
            let mark = match t.status {
                TodoStatus::Pending => "[ ]",
                TodoStatus::InProgress => "[>]",
                TodoStatus::Done => "[x]",
                TodoStatus::Blocked => "[!]",
                TodoStatus::Abandoned => "[-]",
            };
            out.push_str(&format!("  {mark} {}", t.content));
            if let Some(b) = &t.blocker {
                out.push_str(&format!(" (blocked: {b})"));
            }
            out.push('\n');
        }
    }
    out.push_str(&format!("{done}/{total} done"));
    out
}

/// The **write** reply: what happened, how much is left. No list attached — a wrong write means
/// the model reads it again.
pub fn confirm(call: &Call, after: &[TodoPhase]) -> String {
    let (done, total) = counts(after);
    let head = match call.op {
        Op::Init => format!("list replaced: {total} item(s) in {} phase(s)", after.len()),
        Op::Append => format!(
            "added {} item(s) to {:?}",
            call.items.len(),
            call.phase.as_deref().unwrap_or("Tasks")
        ),
        Op::Rm => format!("removed {:?}", call.task.as_deref().unwrap_or_default()),
        op => format!(
            "{}: {:?}",
            op.as_str(),
            call.task.as_deref().unwrap_or_default()
        ),
    };
    format!("{head} ({done}/{total} done)")
}
