//! The `final` tool: a child's draft, its edits, and the one submit.

use anyhow::Result;

use crate::server::agent::tools::bash::edit;
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

use super::answer;

const OP: usize = 0;
const CONTENT: usize = 1;
const OLD: usize = 2;
const NEW: usize = 3;
const RANGE: usize = 4;

pub(crate) const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(&["write", "edit", "read", "submit"]),
        "The operation.",
    ),
    Param::opt("content", Kind::Text, "Full draft for `write`."),
    Param::opt(
        "old",
        Kind::Text,
        "edit: text to replace. MUST match exactly and be unique.",
    ),
    Param::opt(
        "new",
        Kind::Text,
        "edit: the replacement block; empty removes the target.",
    ),
    Param::opt(
        "range",
        Kind::Text,
        "edit: the lines to replace, `12-14` or `12`.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "final",
    about: "Edit the draft a subagent leaves behind. `submit` writes the answer file and ends the \
            task; it must be the only call in its message.",
    params: PARAMS,
    run,
    summary,
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
            let range = args.opt_str(RANGE)?;
            anyhow::ensure!(
                range.is_none() || args.opt_str(OLD)?.is_none(),
                "`range` and `old` pick different ways to locate the edit; give one"
            );
            let (text, _, message) = match range {
                Some(range) => {
                    let (from, to) = edit::parse_range(&range)?;
                    let block = args.opt_str(NEW)?.unwrap_or_default();
                    edit::mass_text(&draft.read()?, from, to, &block)?
                }
                None => edit::exact_text(&draft.read()?, &args.str(OLD)?, &args.str(NEW)?)?,
            };
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

/// One line for a history listing: the operation.
fn summary(call: &CallRecord) -> String {
    let op = summary::field(call, "op").unwrap_or_default();
    summary::clip(&op, 96)
}
