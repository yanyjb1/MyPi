//! The table: fourteen tools, one line each. The roster and the front door both derive from it, so
//! the two readers cannot disagree.

use super::bash::{alias, bash as bash_tool, cd, context, edit, read, write};
use super::kairos;
use super::mcp;
use super::memo::{memo as memo_tool, todo};
use super::spec::{Env, Tool};
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
pub(super) fn spec(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Table → profile gate → the model's manual; request puts it in the request body.
pub fn roster(env: &Env) -> Vec<ToolDef> {
    TOOLS
        .iter()
        .filter(|t| env.allows(t.name))
        .filter(|t| available(env, t.name))
        .map(Tool::definition)
        .collect()
}

/// The session-shaped gate: `task` is parent-only, `final` child-only, `kairos` parent-only.
pub(crate) fn available(env: &Env, name: &str) -> bool {
    match name {
        "kairos" => {
            env.inbox.jobs.is_some() && !env.inbox.subagent.as_ref().is_some_and(|s| s.is_child())
        }
        "task" => env.inbox.subagent.as_ref().is_some_and(|s| s.is_parent()),
        "final" => env.inbox.subagent.as_ref().is_some_and(|s| s.is_child()),
        "mcp" => env.mcp.is_some(),
        _ => true,
    }
}
