//! The `kairos` tool: accept one background operation and return its receipt.

use anyhow::Result;

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

mod command;
mod duration;
mod service;
mod text;

pub(crate) const TYPE: usize = 0;
pub(crate) const KIND: usize = 1;
pub(crate) const CONTENT: usize = 2;
pub(crate) const TIMEOUT: usize = 3;
pub(crate) const DELAY: usize = 4;
pub(crate) const EXPR: usize = 5;
pub(crate) const COUNT: usize = 6;
pub(crate) const ID: usize = 7;

pub(crate) const PARAMS: &[Param] = &[
    Param::new(
        "type",
        Kind::Choice(&[
            "now", "after", "every", "list", "status", "result", "cancel",
        ]),
        "Which operation this is.",
    ),
    Param::df(
        "kind",
        Kind::Choice(&["command", "text"]),
        "What the task is: `command` runs a shell command, `text` wakes with the content.",
        "command",
    ),
    Param::new("content", Kind::Text, "The command or wake text."),
    Param::new("timeout", Kind::Text, "Command timeout, e.g. `10min`."),
    Param::new("delay", Kind::Text, "Delay, e.g. `10min`."),
    Param::new("expr", Kind::Text, "Five-field cron."),
    Param::df("count", Kind::Number, "Runs; -1 is unlimited.", "3"),
    Param::new("id", Kind::Text, "Task, schedule, or run id."),
];

pub(crate) const TOOL: Tool = Tool {
    name: "kairos",
    about: "Hold one background task and return at once. `now` starts a command immediately, `after` \
            waits for a delay, `every` repeats on a five-field cron, `list` shows the held tasks, \
            `status` reports one, `result` hands back what it left, `cancel` stops it. A text task \
            only takes `after` or `every`. The receipt names the task; a wake reports when it is \
            done. Subagents cannot use this tool.",
    fields: "type is always needed. now: content (kind=command only). after: delay and content. \
            every: expr and content, count limits the runs. status, result, cancel: id. list: \
            nothing else. kind and timeout apply to command tasks.",
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
