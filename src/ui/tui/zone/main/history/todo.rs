//! The task list: the server's starting list, then one increment per `todo` call.
//!
//! The server replays every stored call; the front end starts from that replay and applies only
//! the calls whose result landed after the start. The result's memo is not read.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::server::agent::tools::TodoPhase;
use crate::server::storage::BlockKind;
use crate::ui::session::View;

/// The task list and the phases the newest call touched.
#[derive(Default)]
pub struct TaskList {
    phases: Vec<TodoPhase>,
    touched: Vec<String>,
    /// Call ids already accounted for: the start's calls, and every call whose result landed.
    seen: HashSet<String>,
    /// The start has been taken from `view.todo` (or there was nothing to take).
    started: bool,
}

impl TaskList {
    /// Take the server's start once, then apply the calls whose result has landed.
    pub fn sync(&mut self, view: &View<'_>) {
        let requests = requests(view);
        let results = results(view);
        if !self.started && (!view.todo.is_empty() || requests.is_empty()) {
            self.phases = view.todo.to_vec();
            self.touched.clear();
            // The start already carries every call whose result is in hand. A call still in
            // flight is not in the start yet; leave it unseen so its result applies it.
            for (id, _) in &requests {
                if results.contains_key(id) {
                    self.seen.insert(id.to_owned());
                }
            }
            self.started = true;
        }
        for (id, args) in requests {
            if self.seen.contains(&id) {
                continue;
            }
            let Some(ok) = results.get(&id) else {
                continue;
            };
            self.seen.insert(id);
            if !*ok {
                self.touched.clear();
                continue;
            }
            let Some(call) = Call::read(&args) else {
                self.touched.clear();
                continue;
            };
            self.touched = self.apply(&call).unwrap_or_default();
        }
    }

    pub fn phases(&self) -> &[TodoPhase] {
        &self.phases
    }

    pub fn touched(&self) -> &[String] {
        &self.touched
    }

    pub fn clear(&mut self) {
        self.phases.clear();
        self.touched.clear();
        self.seen.clear();
        self.started = false;
    }

    /// Apply one call. `None` = the call was rejected as a whole; the list is unchanged.
    fn apply(&mut self, call: &Call) -> Option<Vec<String>> {
        if call.op == Op::View {
            return Some(Vec::new());
        }
        let mut next = if call.op == Op::Init {
            Vec::new()
        } else {
            self.phases.clone()
        };
        let mut touched = Vec::new();
        let mut errors = 0usize;
        match call.op {
            Op::View => unreachable!("returned above"),
            Op::Init => {
                next = if call.list.is_empty() {
                    vec![TodoPhase {
                        name: "Tasks".to_string(),
                        tasks: call.items.iter().map(|c| task(c, "pending")).collect(),
                    }]
                } else {
                    call.list
                        .iter()
                        .map(|(name, items)| TodoPhase {
                            name: name.clone(),
                            tasks: items.iter().map(|c| task(c, "pending")).collect(),
                        })
                        .collect()
                };
                touched = next.iter().map(|p| p.name.clone()).collect();
            }
            Op::Append => {
                let phase = call.phase.clone().unwrap_or_else(|| "Tasks".to_string());
                let at = match next.iter().position(|p| p.name == phase) {
                    Some(at) => at,
                    None => {
                        next.push(TodoPhase {
                            name: phase.clone(),
                            tasks: Vec::new(),
                        });
                        next.len() - 1
                    }
                };
                for content in &call.items {
                    if next[at].tasks.iter().any(|t| &t.content == content) {
                        errors += 1;
                        continue;
                    }
                    next[at].tasks.push(task(content, "pending"));
                }
                touched.push(phase);
            }
            Op::Rm => {
                let content = call.task.as_deref().unwrap_or_default();
                let mut removed = false;
                for phase in &mut next {
                    let before = phase.tasks.len();
                    phase.tasks.retain(|t| t.content != content);
                    if phase.tasks.len() != before {
                        removed = true;
                        touched.push(phase.name.clone());
                    }
                }
                if !removed {
                    errors += 1;
                }
                next.retain(|p| !p.tasks.is_empty());
            }
            Op::Start | Op::Done | Op::Drop | Op::Block | Op::Unblock => {
                let content = call.task.as_deref().unwrap_or_default();
                let mut found = false;
                for phase in &mut next {
                    for t in &mut phase.tasks {
                        if t.content != content {
                            continue;
                        }
                        found = true;
                        if !touched.contains(&phase.name) {
                            touched.push(phase.name.clone());
                        }
                        match call.op {
                            Op::Start => {
                                t.status = serde_json::from_value(serde_json::json!("in_progress"))
                                    .unwrap();
                                t.blocker = None;
                            }
                            Op::Done => {
                                t.status =
                                    serde_json::from_value(serde_json::json!("done")).unwrap();
                                t.blocker = None;
                            }
                            Op::Drop => {
                                t.status =
                                    serde_json::from_value(serde_json::json!("abandoned")).unwrap();
                                t.blocker = None;
                            }
                            Op::Block => {
                                t.status =
                                    serde_json::from_value(serde_json::json!("blocked")).unwrap();
                                t.blocker = Some(
                                    call.reason
                                        .clone()
                                        .unwrap_or_else(|| "(no reason given)".to_string()),
                                );
                            }
                            Op::Unblock => {
                                t.status =
                                    serde_json::from_value(serde_json::json!("pending")).unwrap();
                                t.blocker = None;
                            }
                            _ => unreachable!("handled above"),
                        }
                    }
                }
                if !found {
                    errors += 1;
                }
                if call.op == Op::Start && found {
                    let in_progress =
                        serde_json::from_value(serde_json::json!("in_progress")).unwrap();
                    for phase in &mut next {
                        for t in &mut phase.tasks {
                            if t.content != content && t.status == in_progress {
                                t.status =
                                    serde_json::from_value(serde_json::json!("pending")).unwrap();
                                if !touched.contains(&phase.name) {
                                    touched.push(phase.name.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        if errors > 0 {
            return None;
        }
        self.phases = next;
        Some(touched)
    }
}

/// One task, built through the type's own serde shape: the front end names neither the task nor
/// its status type, so a server-side re-export is not needed.
fn task<T: serde::de::DeserializeOwned>(content: &str, status: &str) -> T {
    serde_json::from_value(serde_json::json!({
        "content": content,
        "status": status,
    }))
    .expect("a todo task")
}

/// The `todo` requests in display order, with their call id and raw arguments.
fn requests(view: &View<'_>) -> Vec<(String, String)> {
    view.chain()
        .filter_map(|b| {
            if b.kind() != BlockKind::ToolRequest || b.name().is_none_or(|n| n.as_str() != "todo") {
                return None;
            }
            Some((b.call_id()?.to_owned(), b.args()?.to_owned()))
        })
        .collect()
}

/// Which `todo` calls succeeded, by call id.
fn results(view: &View<'_>) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    for b in view.chain() {
        if b.kind() != BlockKind::ToolResult || b.name().is_none_or(|n| n.as_str() != "todo") {
            continue;
        }
        let Some(id) = b.call_id() else {
            continue;
        };
        if let Some(ok) = b.ok() {
            out.insert(id.to_owned(), ok);
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Init,
    Append,
    Start,
    Done,
    Drop,
    Block,
    Unblock,
    Rm,
    View,
}

impl Op {
    fn parse(s: &str) -> Option<Op> {
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
}

/// One call, decoded from the request's arguments.
struct Call {
    op: Op,
    list: Vec<(String, Vec<String>)>,
    items: Vec<String>,
    phase: Option<String>,
    task: Option<String>,
    reason: Option<String>,
}

impl Call {
    fn read(args: &str) -> Option<Call> {
        let raw: Raw = serde_json::from_str(args).ok()?;
        let op = Op::parse(&raw.op)?;
        let call = Call {
            op,
            list: raw.list.into_iter().map(|p| (p.phase, p.items)).collect(),
            items: raw.items,
            phase: raw.phase,
            task: raw.task.filter(|s| !s.trim().is_empty()),
            reason: raw.reason,
        };
        call.validate()?;
        Some(call)
    }

    fn validate(&self) -> Option<()> {
        match self.op {
            Op::Init => {
                if self.list.is_empty() && self.items.is_empty() {
                    return None;
                }
                if self.list.iter().any(|(_, items)| items.is_empty()) {
                    return None;
                }
            }
            Op::Append => {
                if self.items.is_empty() {
                    return None;
                }
            }
            Op::Start | Op::Done | Op::Drop | Op::Block | Op::Unblock | Op::Rm => {
                self.task.as_ref()?;
            }
            Op::View => {}
        }
        Some(())
    }
}

#[derive(Deserialize)]
struct Raw {
    op: String,
    #[serde(default)]
    list: Vec<RawPhase>,
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct RawPhase {
    phase: String,
    #[serde(default)]
    items: Vec<String>,
}
