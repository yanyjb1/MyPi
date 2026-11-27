// The table: every built-in tool, one line each. The roster and the front door both derive from it, so the two readers cannot disagree.

use std::collections::BTreeSet;

use super::cd;
use super::fs::{edit, grep, read, splice, write};
use super::kairos::tool;
use super::mcp;
use super::memo::{context, memo as memo_tool, todo};
#[cfg(feature = "browser")]
use super::search;
use super::shell::{alias, tool as shell_tool};
use super::spec::Tool;
use super::subagent;
use crate::server::agent::request::message::ToolDef;

#[cfg(feature = "browser")]
use super::web;

// Every built-in tool, in the order the model reads them.
pub const TOOLS: &[Tool] = &[
    read::TOOL,
    grep::TOOL,
    edit::TOOL,
    splice::TOOL,
    write::TOOL,
    cd::TOOL,
    shell_tool::TOOL,
    alias::TOOL,
    tool::TOOL,
    subagent::TASK,
    subagent::FINAL,
    mcp::TOOL,
    #[cfg(feature = "browser")]
    web::TOOL,
    #[cfg(feature = "browser")]
    search::TOOL,
    memo_tool::TOOL,
    todo::TOOL,
    context::TOOL,
];

// Look one up by name.
pub(crate) fn spec(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

// The session's list → the model's manuals, in table order. The list is the only gate.
pub fn roster(names: &[String]) -> Vec<ToolDef> {
    let wanted: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    TOOLS
        .iter()
        .filter(|t| wanted.contains(t.name))
        .map(Tool::definition)
        .collect()
}

// What a profile that names no list gets: everything but `edit` and `final`. `splice` is the writer a session starts with, and `final` is a child's alone.
pub fn defaults() -> Vec<String> {
    TOOLS
        .iter()
        .filter(|tool| tool.name != edit::TOOL.name && tool.name != subagent::FINAL.name)
        .map(|tool| tool.name.to_string())
        .collect()
}
