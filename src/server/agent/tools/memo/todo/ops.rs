//! The three ops: replace the list, tick item numbers, look at it.

use anyhow::{Result, anyhow};

use super::state::{TodoItem, counts};
use crate::server::agent::tools::spec::Args;

// Positions in the parameter table; `TOOL` writes it in this order.
pub(super) const TYPE: usize = 0;
pub(super) const ITEMS: usize = 1;
pub(super) const IDS: usize = 2;
pub(super) const DONE: usize = 3;

/// One op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Replace the whole list; empty clears it.
    Set,
    /// Mark the given item numbers done, or not done.
    Tick,
    /// Report the list, change nothing.
    View,
}

impl Op {
    /// The allowed values, for the manual.
    pub const ALL: &'static [&'static str] = &["set", "tick", "view"];

    pub fn parse(s: &str) -> Option<Op> {
        Some(match s {
            "set" => Op::Set,
            "tick" => Op::Tick,
            "view" => Op::View,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Op::Set => "set",
            Op::Tick => "tick",
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
    /// `set`: the whole list, in order.
    pub items: Vec<String>,
    /// `tick`: the item numbers to change, 1-based.
    pub ids: Vec<i64>,
    /// `tick`: mark done (true) or clear the tick (false).
    pub done: bool,
}

impl Call {
    /// Read from the parameter table, then shape-checked.
    pub fn read(args: &Args) -> Result<Call> {
        let name = args.choice(TYPE)?;
        let op = Op::parse(&name)
            .ok_or_else(|| anyhow!("unknown type `{name}` ({})", Op::ALL.join("/")))?;
        let call = Call {
            op,
            items: args.str_list(ITEMS)?,
            ids: numbers(args)?,
            done: args.flag(DONE)?,
        };
        call.validate()?;
        Ok(call)
    }

    /// Check the shape per op — errors the model can act on.
    fn validate(&self) -> Result<()> {
        match self.op {
            Op::Set => anyhow::ensure!(self.ids.is_empty(), "`set` takes `items` only"),
            Op::Tick => anyhow::ensure!(
                !self.ids.is_empty(),
                "missing argument `ids` for type=tick: the item numbers `view` printed, from 1"
            ),
            Op::View => anyhow::ensure!(
                self.items.is_empty() && self.ids.is_empty(),
                "`view` takes nothing"
            ),
        }
        Ok(())
    }
}

/// The item numbers as given: integers, or their decimal spelling.
fn numbers(args: &Args) -> Result<Vec<i64>> {
    let Some(value) = args.opt_any(IDS) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| anyhow!("`ids` must be an array of item numbers"))?;
    let mut out = Vec::with_capacity(array.len());
    for one in array {
        let id = match one {
            serde_json::Value::Number(n) => n.as_i64(),
            serde_json::Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
        .ok_or_else(|| anyhow!("`ids` takes item numbers; got {one}"))?;
        out.push(id);
    }
    Ok(out)
}

/// Apply one call; returns the new list. Errors are **collected**, not fatal: the caller drops the
/// whole batch and every problem comes back in one pass.
pub fn apply(items: &[TodoItem], call: &Call) -> (Vec<TodoItem>, Vec<String>) {
    match call.op {
        Op::View => (items.to_vec(), Vec::new()),
        Op::Set => (installed(call), Vec::new()),
        Op::Tick => {
            let mut next = items.to_vec();
            let mut errors = Vec::new();
            for id in &call.ids {
                match position(*id, next.len()) {
                    Some(at) => next[at].done = call.done,
                    None => errors.push(format!(
                        "no item {id}; the list has {} item(s) — use view to read the numbers",
                        next.len()
                    )),
                }
            }
            if errors.is_empty() {
                (next, errors)
            } else {
                (items.to_vec(), errors)
            }
        }
    }
}

/// The list a `set` installs: every entry fresh, nothing kept from before.
pub fn installed(call: &Call) -> Vec<TodoItem> {
    call.items
        .iter()
        .map(|text| TodoItem {
            text: text.clone(),
            done: false,
        })
        .collect()
}

/// One item number → its index. A number outside the list is no position at all.
fn position(id: i64, len: usize) -> Option<usize> {
    usize::try_from(id)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .filter(|i| *i < len)
}

/// The **view** reply: the whole list with the numbers `tick` takes. An empty list gets a
/// placeholder, not an empty string.
pub fn rendered(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "(the list is empty)".to_string();
    }
    let (done, total) = counts(items);
    let mut out = String::new();
    for (at, item) in items.iter().enumerate() {
        let mark = if item.done { "[x]" } else { "[ ]" };
        out.push_str(&format!("{mark} {}. {}\n", at + 1, item.text));
    }
    out.push_str(&format!("{done}/{total} done"));
    out
}

/// The **write** reply: what happened, how much is left. No list attached — a wrong write means the
/// model reads it again.
pub fn confirm(call: &Call, after: &[TodoItem]) -> String {
    let (done, total) = counts(after);
    let head = match call.op {
        Op::Set => format!("list set: {total} item(s)"),
        Op::Tick => {
            let ids = call
                .ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            format!("{} {ids}", if call.done { "ticked" } else { "unticked" })
        }
        Op::View => "view".to_string(),
    };
    format!("{head} ({done}/{total} done)")
}
