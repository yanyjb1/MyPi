//! The `final` tool: a child's draft, its edits, and the one submit.

use anyhow::Result;

use crate::server::agent::tools::bash::edit;
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::Source;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};

use super::answer;

const OP: usize = 0;
const CONTENT: usize = 1;
const OLD: usize = 2;
const NEW: usize = 3;
const EDITS: usize = 4;

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
    about: "Edit the draft a subagent leaves behind. `submit` writes the answer file and ends the \
            task; it must be the only call in its message.",
    params: PARAMS,
    run,
    summary: None,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let child = env
        .source
        .child(env.session)
        .ok_or_else(|| anyhow::anyhow!("`final` is only available to a subagent"))?;
    let draft = child.draft();
    match args.choice(OP)?.as_str() {
        "write" => {
            draft.write(&args.str(CONTENT)?)?;
            Ok(ToolOutput::text("draft written"))
        }
        "edit" => {
            let (text, _, message) =
                edit::exact_text(&draft.read()?, &args.str(OLD)?, &args.str(NEW)?)?;
            draft.write(&text)?;
            Ok(ToolOutput::text(message))
        }
        "mass_edit" => {
            let rows = args.table(EDITS)?;
            let mut edits = Vec::with_capacity(rows.len());
            for row in &rows {
                edits.push((row.line(0)?, row.str(1)?));
            }
            let (text, _, message) = edit::mass_text(&draft.read()?, &edits)?;
            draft.write(&text)?;
            Ok(ToolOutput::text(message))
        }
        "read" => {
            let text = draft.read()?;
            if text.is_empty() {
                Ok(ToolOutput::text("(empty draft)"))
            } else {
                Ok(ToolOutput::text(text))
            }
        }
        "submit" => {
            anyhow::ensure!(
                env.batch_len == 1,
                "`submit` must be the only call in its message"
            );
            let text = draft.read()?;
            anyhow::ensure!(!text.trim().is_empty(), "the draft is empty");
            answer::write(child.answer(), &text)?;
            child.submit();
            Ok(ToolOutput::text(format!("submitted {}", child.answer().display())).stop())
        }
        _ => unreachable!(),
    }
}
