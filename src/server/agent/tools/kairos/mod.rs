//! The `kairos` tool: accept one background operation and return its receipt.

use anyhow::Result;

use crate::server::agent::tools::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

mod command;
mod service;

pub(crate) const OP: usize = 0;
pub(crate) const TYPE: usize = 1;
pub(crate) const CONTENT: usize = 2;
pub(crate) const TIMEOUT: usize = 3;
pub(crate) const DELAY: usize = 4;
pub(crate) const EXPR: usize = 5;
pub(crate) const COUNT: usize = 6;
pub(crate) const ID: usize = 7;
pub(crate) const NOTIFY: usize = 8;

pub(crate) const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(&[
            "now", "after", "every", "list", "status", "result", "cancel",
        ]),
        "The operation.",
    ),
    Param::df(
        "type",
        Kind::Choice(&["command", "text"]),
        "`command` runs a shell command; `text` wakes with the content.",
        "command",
    ),
    Param::opt("content", Kind::Text, "The command or wake text."),
    Param::opt("timeout", Kind::Text, "Command timeout, e.g. `10min`."),
    Param::opt("delay", Kind::Text, "Delay for `after`, e.g. `10min`."),
    Param::opt("expr", Kind::Text, "Five-field cron for `every`."),
    Param::df(
        "count",
        Kind::Number,
        "Runs for `every`; -1 is unlimited.",
        "3",
    ),
    Param::opt("id", Kind::Text, "Task, schedule, or run id."),
    Param::df(
        "notify",
        Kind::Flag,
        "Wake when `cancel` removes a task.",
        "false",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "kairos",
    about: "Accept one background operation and return at once. `now` runs immediately, `after` waits\n\
            for a delay, and `every` repeats on a five-field cron. A finished command wakes you; fetch its\n\
            output with `kairos result`. Subagents cannot use this tool.",
    params: PARAMS,
    run,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    service::run(env, args)
}
