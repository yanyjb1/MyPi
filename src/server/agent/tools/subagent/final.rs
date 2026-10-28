//! The `final` tool: the child's draft and its one submit.

use anyhow::Result;

use crate::server::agent::tools::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

const RECORD: &[Param] = &[
    Param::req("line", Kind::Line, "Line number, 1-based."),
    Param::req("text", Kind::Text, "New content for that line."),
];

pub(crate) const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(&["write", "edit", "mass_edit", "read", "submit"]),
        "The draft operation.",
    ),
    Param::opt("content", Kind::Text, "Full draft for `write`."),
    Param::opt("old", Kind::Text, "Exact text for `edit`."),
    Param::opt("new", Kind::Text, "Replacement text for `edit`."),
    Param::opt(
        "edits",
        Kind::Table(RECORD),
        "Line replacements for `mass_edit`.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "final",
    about: "Edit the draft a subagent leaves behind. `submit` writes the final file and ends the \
            subagent; it must be the only call in its message.",
    params: PARAMS,
    run,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let subagent = env
        .inbox
        .subagent
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("`final` is unavailable in this session"))?;
    subagent.final_op(args)
}
