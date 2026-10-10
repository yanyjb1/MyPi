//! The table: eleven tools, one line each. The roster and the front door both derive from it, so the
//! two readers cannot disagree (rationale in `src/server/agent/README.md`).

use super::context;
use super::spec::{Env, Tool};
use crate::server::agent::request::message::ToolDef;

use super::bash;
use super::cd;
use super::edit;
use super::mass_edit;
use super::read;
use super::todo;
use super::write;

#[cfg(feature = "web")]
use super::browser;
#[cfg(feature = "web")]
use super::fetch;
#[cfg(feature = "web")]
use super::search;

/// Every built-in tool, in the order the model reads them. With `web` off those three rows are
/// absent from the table itself, not present-but-unavailable.
pub const TOOLS: &[Tool] = &[
    read::TOOL,
    edit::TOOL,
    write::TOOL,
    cd::TOOL,
    bash::TOOL,
    mass_edit::TOOL,
    #[cfg(feature = "web")]
    fetch::TOOL,
    #[cfg(feature = "web")]
    browser::TOOL,
    #[cfg(feature = "web")]
    search::TOOL,
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
        .map(Tool::definition)
        .collect()
}
