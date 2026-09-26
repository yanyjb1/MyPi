//! The `todo` tool: the model's list, kept **outside** the model's head.
//!
//! # Why the list is session state, not a tool argument
//!
//! A todo list whose state lived in the model's context would be pointless: the
//! model would have to hold the whole list to re-send it, and a compaction could
//! take it away. So the shape is the opposite:
//!
//! * the model sends an **operation** plus the one item it is talking about
//!   (`{"op":"done","task":"接 /switch"}`) — it only has to remember the line it
//!   just wrote, which is right there in the previous result;
//! * the **server** holds the list (as [`crate::server::entry::Entry::Todo`],
//!   which survives resume, branch switches and compaction);
//! * every result carries the **whole list** back, so the model can always
//!   re-read its own memo, and `{"op":"view"}` asks for it explicitly.
//!
//! Corresponds to omp's `packages/coding-agent/src/tools/todo.ts` (same nine
//! operations, same "any error discards the whole batch" rule).
//!
//! # The one rule that matters
//!
//! A batch with any error is **discarded wholesale**: persisting a half-applied
//! batch makes the natural retry hit "already done" for the operations that did
//! land. State stays exactly where it was, and the model gets told why.

use anyhow::{Result, anyhow};

use crate::server::entry::{TodoPhase, TodoStatus, TodoTask, todo_counts};

/// The tool layer's view of the list: what it reads, and what it produced.
///
/// A shared slot (like the `cd` tool's cwd slot) so neither layer has to know
/// the other: the tool writes, the turn runner publishes.
#[derive(Debug, Default)]
pub struct TodoState {
    /// The list as it stands right now (seeded from the session's last
    /// `Entry::Todo` before the round).
    pub current: Vec<TodoPhase>,
    /// The list this round's calls produced and nobody has published yet.
    /// `None` = unchanged (a `view`, or no call at all).
    pub pending: Option<Vec<TodoPhase>>,
}

/// One todo operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoOp {
    /// Replace the whole list (`list` = phases, or flat `items`).
    Init,
    /// Add items to a phase (creating it if needed).
    Append,
    /// Mark one item as being worked on now.
    Start,
    /// Mark one item finished.
    Done,
    /// Give up on one item (not the same as "not done yet").
    Drop,
    /// One item cannot proceed; `reason` says why.
    Block,
    /// A blocked item is unblocked (back to pending).
    Unblock,
    /// Remove one item outright.
    Rm,
    /// Report the current list without changing it.
    View,
}

impl TodoOp {
    fn parse(s: &str) -> Option<TodoOp> {
        Some(match s {
            "init" => TodoOp::Init,
            "append" => TodoOp::Append,
            "start" => TodoOp::Start,
            "done" => TodoOp::Done,
            "drop" => TodoOp::Drop,
            "block" => TodoOp::Block,
            "unblock" => TodoOp::Unblock,
            "rm" => TodoOp::Rm,
            "view" => TodoOp::View,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TodoOp::Init => "init",
            TodoOp::Append => "append",
            TodoOp::Start => "start",
            TodoOp::Done => "done",
            TodoOp::Drop => "drop",
            TodoOp::Block => "block",
            TodoOp::Unblock => "unblock",
            TodoOp::Rm => "rm",
            TodoOp::View => "view",
        }
    }

    /// A read changes nothing (no state write, no entry).
    pub fn is_read_only(self) -> bool {
        matches!(self, TodoOp::View)
    }
}

/// One parsed call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoArgs {
    pub op: TodoOp,
    /// `init`: whole phases.
    pub list: Vec<(String, Vec<String>)>,
    /// `init` (flat) / `append`: item texts.
    pub items: Vec<String>,
    /// Which phase an `append` goes to.
    pub phase: Option<String>,
    /// The item an operation acts on, **verbatim** as written before.
    pub task: Option<String>,
    /// `block`: why.
    pub reason: Option<String>,
}

impl TodoArgs {
    /// Parse the raw argument string.
    ///
    /// Lenient on purpose about *shape*, strict about *meaning*: a model that
    /// sends a bare list instead of `{op, list}` gets a readable error naming
    /// the accepted shapes, not a serde message.
    pub fn parse(raw: &str) -> Result<TodoArgs> {
        let v: serde_json::Value =
            serde_json::from_str(raw).map_err(|e| anyhow!("arguments 不是合法 JSON：{e}"))?;
        let op_name = v
            .get("op")
            .and_then(|o| o.as_str())
            .ok_or_else(|| anyhow!("缺 `op`（init/append/start/done/drop/block/unblock/rm/view）"))?;
        let op = TodoOp::parse(op_name)
            .ok_or_else(|| anyhow!("未知的 op `{op_name}`（init/append/start/done/drop/block/unblock/rm/view）"))?;

        let mut list = Vec::new();
        if let Some(arr) = v.get("list").and_then(|l| l.as_array()) {
            for entry in arr {
                let name = entry
                    .get("phase")
                    .and_then(|p| p.as_str())
                    .ok_or_else(|| anyhow!("list 里每一项都要有 `phase` 名字"))?
                    .to_string();
                let items = entry
                    .get("items")
                    .and_then(|i| i.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str())
                            .map(String::from)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                list.push((name, items));
            }
        }
        let items: Vec<String> = v
            .get("items")
            .and_then(|i| i.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let text = |key: &str| -> Option<String> {
            v.get(key)
                .and_then(|x| x.as_str())
                .map(String::from)
                .filter(|s| !s.trim().is_empty())
        };
        let args = TodoArgs {
            op,
            list,
            items,
            phase: text("phase"),
            task: text("task"),
            reason: text("reason"),
        };
        args.validate()?;
        Ok(args)
    }

    /// Shape checks per operation — the errors the model can actually act on.
    fn validate(&self) -> Result<()> {
        match self.op {
            TodoOp::Init => {
                anyhow::ensure!(
                    !self.list.is_empty() || !self.items.is_empty(),
                    "init 需要 `list: [{{phase, items}}]` 或 `items: [..]`"
                );
                anyhow::ensure!(
                    self.list.iter().all(|(_, i)| !i.is_empty()),
                    "init 里每个阶段至少一项"
                );
            }
            TodoOp::Append => {
                anyhow::ensure!(!self.items.is_empty(), "append 需要 `items: [..]`");
            }
            TodoOp::Start | TodoOp::Done | TodoOp::Drop | TodoOp::Block | TodoOp::Unblock
            | TodoOp::Rm => {
                anyhow::ensure!(
                    self.task.is_some(),
                    "{} 需要 `task`：那一项的原文（照抄你写下时的那句）",
                    self.op.as_str()
                );
            }
            TodoOp::View => {}
        }
        Ok(())
    }
}

/// Apply one call to `phases`, returning the new list.
///
/// Errors are **collected**, not raised: a batch with any error is discarded by
/// the caller, and the messages name every problem at once (so one round trip
/// fixes them all).
pub fn apply(phases: &[TodoPhase], args: &TodoArgs) -> (Vec<TodoPhase>, Vec<String>) {
    let mut next = phases.to_vec();
    let mut errors = Vec::new();
    match args.op {
        TodoOp::View => {}
        TodoOp::Init => {
            next = if !args.list.is_empty() {
                args.list
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
                    name: "任务".to_string(),
                    tasks: args
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
        TodoOp::Append => {
            let phase = args.phase.clone().unwrap_or_else(|| "任务".to_string());
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
            for content in &args.items {
                if target.tasks.iter().any(|t| &t.content == content) {
                    errors.push(format!("「{content}」已经在清单里了"));
                    continue;
                }
                target.tasks.push(TodoTask {
                    content: content.clone(),
                    status: TodoStatus::Pending,
                    blocker: None,
                });
            }
        }
        TodoOp::Rm => {
            let task = args.task.as_deref().unwrap_or_default();
            let mut removed = false;
            for phase in next.iter_mut() {
                let before = phase.tasks.len();
                phase.tasks.retain(|t| t.content != task);
                removed |= phase.tasks.len() != before;
            }
            if !removed {
                errors.push(format!("清单里没有「{task}」"));
            }
            next.retain(|p| !p.tasks.is_empty());
        }
        // Every status operation goes through the same lookup.
        TodoOp::Start | TodoOp::Done | TodoOp::Drop | TodoOp::Block | TodoOp::Unblock => {
            let task = args.task.as_deref().unwrap_or_default();
            let mut found = false;
            for phase in next.iter_mut() {
                for t in phase.tasks.iter_mut() {
                    if t.content != task {
                        continue;
                    }
                    found = true;
                    match args.op {
                        TodoOp::Start => {
                            t.status = TodoStatus::InProgress;
                            t.blocker = None;
                        }
                        TodoOp::Done => {
                            t.status = TodoStatus::Done;
                            t.blocker = None;
                        }
                        TodoOp::Drop => {
                            t.status = TodoStatus::Abandoned;
                            t.blocker = None;
                        }
                        TodoOp::Block => {
                            t.status = TodoStatus::Blocked;
                            t.blocker = Some(
                                args.reason
                                    .clone()
                                    .unwrap_or_else(|| "（未说明原因）".to_string()),
                            );
                        }
                        TodoOp::Unblock => {
                            t.status = TodoStatus::Pending;
                            t.blocker = None;
                        }
                        _ => unreachable!("handled above"),
                    }
                }
            }
            if !found {
                errors.push(format!(
                    "清单里没有「{task}」——`task` 必须与写下时逐字一致（用 view 看当前清单）"
                ));
            }
            // One thing at a time: starting an item retires whatever was
            // in progress. A list with three "current" items tells nobody
            // anything.
            if args.op == TodoOp::Start && found {
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
        // Discarded wholesale — see the module docs.
        return (phases.to_vec(), errors);
    }
    (next, errors)
}

/// The list as the **model** reads it: plain text, whole list every time.
pub fn summary_text(phases: &[TodoPhase]) -> String {
    if phases.is_empty() {
        return "（清单是空的）".to_string();
    }
    let (done, total) = todo_counts(phases);
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
                out.push_str(&format!("（卡住：{b}）"));
            }
            out.push('\n');
        }
    }
    out.push_str(&format!("{done}/{total} done"));
    out
}

/// The `details` payload the card renders from.
pub fn details(phases: &[TodoPhase], op: TodoOp) -> serde_json::Value {
    let (done, total) = todo_counts(phases);
    serde_json::json!({
        "kind": "todo",
        "op": op.as_str(),
        "phases": phases,
        "done": done,
        "total": total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phases_of(list: &[(&str, &[&str])]) -> Vec<TodoPhase> {
        list.iter()
            .map(|(name, items)| TodoPhase {
                name: name.to_string(),
                tasks: items
                    .iter()
                    .map(|c| TodoTask {
                        content: c.to_string(),
                        status: TodoStatus::Pending,
                        blocker: None,
                    })
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn init_builds_phases_or_a_flat_list() {
        let args = TodoArgs::parse(r#"{"op":"init","list":[{"phase":"A","items":["x","y"]}]}"#)
            .unwrap();
        let (next, errs) = apply(&[], &args);
        assert!(errs.is_empty());
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].name, "A");
        assert_eq!(next[0].tasks.len(), 2);
        assert!(next[0].tasks.iter().all(|t| t.status == TodoStatus::Pending));

        let args = TodoArgs::parse(r#"{"op":"init","items":["只有一项"]}"#).unwrap();
        let (next, _) = apply(&[], &args);
        assert_eq!(next.len(), 1, "平铺形状落进一个默认阶段");
        assert_eq!(next[0].tasks[0].content, "只有一项");
    }

    #[test]
    fn status_ops_look_items_up_by_their_verbatim_text() {
        let cur = phases_of(&[("A", &["甲", "乙"])]);
        let done = TodoArgs::parse(r#"{"op":"done","task":"甲"}"#).unwrap();
        let (next, errs) = apply(&cur, &done);
        assert!(errs.is_empty());
        assert_eq!(next[0].tasks[0].status, TodoStatus::Done);
        assert_eq!(next[0].tasks[1].status, TodoStatus::Pending);

        // A typo is an error the model can act on, not a silent no-op.
        let bad = TodoArgs::parse(r#"{"op":"done","task":"丙"}"#).unwrap();
        let (same, errs) = apply(&cur, &bad);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("丙"));
        assert_eq!(same, cur, "有错就整批丢弃，状态不动");
    }

    #[test]
    fn starting_an_item_retires_the_previous_one() {
        let cur = phases_of(&[("A", &["甲", "乙"])]);
        let start = TodoArgs::parse(r#"{"op":"start","task":"甲"}"#).unwrap();
        let (next, _) = apply(&cur, &start);
        assert_eq!(next[0].tasks[0].status, TodoStatus::InProgress);
        let start = TodoArgs::parse(r#"{"op":"start","task":"乙"}"#).unwrap();
        let (next, _) = apply(&next, &start);
        assert_eq!(next[0].tasks[1].status, TodoStatus::InProgress);
        assert_eq!(
            next[0].tasks[0].status,
            TodoStatus::Pending,
            "一次只能有一项在跑"
        );
    }

    #[test]
    fn block_carries_its_reason_and_unblock_clears_it() {
        let cur = phases_of(&[("A", &["甲"])]);
        let block = TodoArgs::parse(r#"{"op":"block","task":"甲","reason":"等上游"}"#).unwrap();
        let (next, _) = apply(&cur, &block);
        assert_eq!(next[0].tasks[0].status, TodoStatus::Blocked);
        assert_eq!(next[0].tasks[0].blocker.as_deref(), Some("等上游"));
        assert!(summary_text(&next).contains("卡住：等上游"));

        let unblock = TodoArgs::parse(r#"{"op":"unblock","task":"甲"}"#).unwrap();
        let (next, _) = apply(&next, &unblock);
        assert_eq!(next[0].tasks[0].status, TodoStatus::Pending);
        assert!(next[0].tasks[0].blocker.is_none());
    }

    #[test]
    fn append_adds_to_a_phase_and_refuses_duplicates() {
        let cur = phases_of(&[("A", &["甲"])]);
        let add = TodoArgs::parse(r#"{"op":"append","phase":"A","items":["乙"]}"#).unwrap();
        let (next, errs) = apply(&cur, &add);
        assert!(errs.is_empty());
        assert_eq!(next[0].tasks.len(), 2);

        let dup = TodoArgs::parse(r#"{"op":"append","phase":"A","items":["甲"]}"#).unwrap();
        let (same, errs) = apply(&next, &dup);
        assert_eq!(errs.len(), 1);
        assert_eq!(same, next);
    }

    #[test]
    fn rm_removes_the_item_and_an_empty_phase_with_it() {
        let cur = phases_of(&[("A", &["甲"]), ("B", &["乙"])]);
        let rm = TodoArgs::parse(r#"{"op":"rm","task":"甲"}"#).unwrap();
        let (next, errs) = apply(&cur, &rm);
        assert!(errs.is_empty());
        assert_eq!(next.len(), 1, "空阶段跟着走");
        assert_eq!(next[0].name, "B");
    }

    #[test]
    fn view_changes_nothing() {
        let cur = phases_of(&[("A", &["甲"])]);
        let view = TodoArgs::parse(r#"{"op":"view"}"#).unwrap();
        let (next, errs) = apply(&cur, &view);
        assert_eq!(next, cur);
        assert!(errs.is_empty());
        assert!(view.op.is_read_only());
    }

    #[test]
    fn malformed_calls_say_what_is_missing() {
        // 不是 JSON
        assert!(TodoArgs::parse("不是 json").unwrap_err().to_string().contains("JSON"));
        // 缺 op
        assert!(TodoArgs::parse(r#"{"task":"甲"}"#).unwrap_err().to_string().contains("op"));
        // 未知 op
        let e = TodoArgs::parse(r#"{"op":"finish"}"#).unwrap_err().to_string();
        assert!(e.contains("finish"), "{e}");
        // 缺 task
        let e = TodoArgs::parse(r#"{"op":"done"}"#).unwrap_err().to_string();
        assert!(e.contains("task"), "{e}");
        // 空 items
        assert!(TodoArgs::parse(r#"{"op":"append","items":[]}"#).is_err());
        assert!(TodoArgs::parse(r#"{"op":"init"}"#).is_err());
    }

    #[test]
    fn the_summary_always_carries_the_whole_list() {
        // 模型就是靠这个"重读自己的备忘录"——少了任何一项都算丢记忆。
        let mut cur = phases_of(&[("阶段一", &["甲", "乙"]), ("阶段二", &["丙"])]);
        cur[0].tasks[0].status = TodoStatus::Done;
        cur[0].tasks[1].status = TodoStatus::InProgress;
        let text = summary_text(&cur);
        for needle in ["阶段一", "阶段二", "甲", "乙", "丙", "[x]", "[>]", "[ ]", "1/3 done"] {
            assert!(text.contains(needle), "缺 {needle}：\n{text}");
        }
    }
}
