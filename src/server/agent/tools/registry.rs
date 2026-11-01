//! The table: fourteen tools, one line each. The roster and the front door both derive from it, so
//! the two readers cannot disagree.

use std::collections::BTreeSet;

use super::bash::{alias, bash as bash_tool, cd, context, edit, read, write};
use super::kairos;
use super::mcp;
use super::memo::{memo as memo_tool, todo};
use super::spec::Tool;
use super::subagent;
use crate::server::agent::request::message::ToolDef;

#[cfg(feature = "web")]
use super::web;

/// Every built-in tool, in the order the model reads them. With `web` off that row is absent from
/// the table itself, not present-but-unavailable.
pub const TOOLS: &[Tool] = &[
    read::TOOL,
    edit::TOOL,
    write::TOOL,
    cd::TOOL,
    bash_tool::TOOL,
    alias::TOOL,
    kairos::TOOL,
    subagent::TASK,
    subagent::FINAL,
    mcp::TOOL,
    #[cfg(feature = "web")]
    web::TOOL,
    memo_tool::TOOL,
    todo::TOOL,
    context::TOOL,
];

/// Look one up by name.
pub(crate) fn spec(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The session's list → the model's manuals, in table order. The list is the only gate.
pub fn roster(names: &[String]) -> Vec<ToolDef> {
    let wanted: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    TOOLS
        .iter()
        .filter(|t| wanted.contains(t.name))
        .map(Tool::definition)
        .collect()
}
