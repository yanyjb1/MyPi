//! The `kairos` tool: accept background operations and return their receipts.

use anyhow::Result;

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

use super::service;

pub(crate) const TYPE: usize = 0;
pub(crate) const TASKS: usize = 1;
pub(crate) const ID: usize = 2;

/// One task a starting operation carries.
pub(crate) const TASK_FIELDS: &[Param] = &[
    Param::new("content", Kind::Text, "The command or wake text."),
    Param::df(
        "kind",
        Kind::Choice(&["command", "text"]),
        "What the task is: `command` runs a shell command, `text` wakes with the content.",
        "command",
    ),
    Param::new("timeout", Kind::Text, "Command timeout, e.g. `10min`."),
    Param::new("delay", Kind::Text, "Delay, e.g. `10min`."),
    Param::new("expr", Kind::Text, "Five-field cron."),
    Param::df("count", Kind::Number, "Runs; -1 is unlimited.", "3"),
];

pub(crate) const PARAMS: &[Param] = &[
    Param::new(
        "type",
        Kind::Choice(&[
            "now", "after", "every", "list", "status", "result", "cancel",
        ]),
        "`now` starts tasks immediately, `after` waits for a delay, `every` repeats on a \
         five-field cron, `list` shows the held tasks, `status` reports one, `result` hands back \
         what it left, `cancel` stops it.",
    ),
    Param::new(
        "tasks",
        Kind::Objects(TASK_FIELDS),
        "type=now/after/every: one item per task, in this order. One item's failure never stops \
         the ones after it.",
    ),
    Param::new("id", Kind::Text, "Task, schedule, or run id."),
];

pub(crate) const TOOL: Tool = Tool {
    name: "kairos",
    about: "Hold background tasks and return their ids.",
    fields: "type is always needed. now: tasks (kind=command only). after: every task names \
             delay and content. every: every task names expr and content, and count limits the \
             runs. status, result, cancel: id. list: nothing else. kind and timeout apply to \
             command tasks; text tasks only take `after` or `every`.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    service::run(env, args)
}

/// One line for a history listing: the operation.
fn summary(call: &CallRecord) -> String {
    summary::field(call, "type").unwrap_or_default()
}
