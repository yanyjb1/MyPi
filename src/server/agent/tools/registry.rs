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

/// What the session looks like, for the two gates that depend on it.
#[derive(Debug, Clone, Copy)]
pub struct Availability {
    pub child: bool,
    pub jobs: bool,
    pub mcp: bool,
}

/// Look one up by name.
pub(crate) fn spec(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The session's list → the model's manuals, in table order.
pub fn roster(names: &[String], shape: Availability) -> Vec<ToolDef> {
    let wanted: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    TOOLS
        .iter()
        .filter(|t| wanted.contains(t.name))
        .filter(|t| available(t.name, &shape))
        .map(Tool::definition)
        .collect()
}

/// The session-shaped gate: `task` is main-only, `final` is sub-only, `kairos` is main-only and
/// needs the job service, `mcp` needs a bridge.
pub(crate) fn available(name: &str, shape: &Availability) -> bool {
    match name {
        "kairos" => shape.jobs && !shape.child,
        "task" => !shape.child,
        "final" => shape.child,
        "mcp" => shape.mcp,
        _ => true,
    }
}
