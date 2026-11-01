//! The `todo` tool: a short checklist kept outside the model's head. It never fails the turn by
//! bookkeeping: writing replaces the list, ticking takes the numbers `view` prints.

use anyhow::{Result, anyhow};

use super::ops;
use super::replay;
use crate::server::agent::tools::summary;
use crate::server::agent::tools::{
    Args, CallRecord, Env, Kind, Param, Tool, ToolOutput, ToolProgress,
};

/// The marker a `set` result carries: the newest one is where a replay starts.
pub(crate) const RESET: &str = "\"todo_reset\":true";

pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(ops::Op::ALL),
        "set: replace the whole list (empty clears it); tick: change the given items; view: read it.",
    ),
    Param::opt(
        "items",
        Kind::TextList,
        "set: the whole list, in order. An empty array clears the list.",
    ),
    Param::opt(
        "ids",
        Kind::Numbers,
        "tick: the item numbers `view` printed, from 1.",
    ),
    Param::df(
        "done",
        Kind::Flag,
        "tick: mark done (default true); false clears the tick.",
        "true",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "todo",
    about: "Keep a short checklist for a long task, outside your head; it survives compaction.\n\
            `set` replaces the whole list in order; an empty `items` clears it. `tick` marks the given item\n\
            numbers done, or clears their tick with `done: false`. `view` prints the list with the numbers\n\
            `tick` takes. Numbers are positions, 1-based; an unknown number fails the call and changes nothing.",
    params: PARAMS,
    run,
    summary: Some(summary),
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let call = ops::Call::read(args)?;
    if call.op == ops::Op::Set {
        // A replacement needs nothing from the store: the call is the whole list.
        let after = ops::installed(&call);
        let text = ops::confirm(&call, &after);
        return Ok(ToolOutput::with_memo(
            text,
            serde_json::json!({ "items": after }),
        ));
    }
    let before = replay::current(env.source.as_ref(), env.session)?;
    if call.op.is_read_only() {
        return Ok(ToolOutput::text(ops::rendered(&before)));
    }

    let (after, errors) = ops::apply(&before, &call);
    if !errors.is_empty() {
        return Err(anyhow!(
            "{} not applied; the list is unchanged:\n- {}",
            call.op.as_str(),
            errors.join("\n- ")
        ));
    }

    let text = ops::confirm(&call, &after);
    let memo = serde_json::json!({ "items": after });
    Ok(ToolOutput::with_memo(text, memo))
}

/// One line for a history listing: the operation and what it touched.
fn summary(call: &CallRecord) -> String {
    let op = summary::field(call, "op").unwrap_or_default();
    match summary::field(call, "items") {
        Some(_) => summary::clip(&format!("{op} list"), 96),
        None => summary::clip(&format!("{op} {}", numbers_of(call)), 96),
    }
}

/// The item numbers a `tick` call names, for the summary.
fn numbers_of(call: &CallRecord) -> String {
    let Ok(args) = serde_json::from_str::<serde_json::Value>(&call.args) else {
        return String::new();
    };
    let Some(ids) = args.get("ids").and_then(|v| v.as_array()) else {
        return String::new();
    };
    ids.iter()
        .filter_map(|v| v.as_i64())
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(" ")
}
