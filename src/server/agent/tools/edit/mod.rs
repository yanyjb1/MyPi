//! edit: replace one unique run of text.

use anyhow::{Context as _, Result, anyhow};
use serde_json::json;

use super::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

const PATH: usize = 0;
const OLD: usize = 1;
const NEW: usize = 2;

pub(super) const PARAMS: &[Param] = &[
    Param::req("path", Kind::Text, "File path. `./` and `../` allowed."),
    Param::req(
        "old",
        Kind::Text,
        "Text to replace. MUST match exactly and be unique in the file.",
    ),
    Param::req("new", Kind::Text, "Replacement text."),
];

pub(super) const TOOL: Tool = Tool {
    name: "edit",
    about: "Replace one run of text with another. `old` MUST match the file character for character and appear exactly once.\n\
            Use it when the target has unique surrounding context; when repeated lines make it ambiguous, use mass_edit.\n\
            Only files inside the workspace and /tmp.",
    params: PARAMS,
    run,
};

/// Find `old` in the file — it MUST appear exactly once — and swap in `new`.
fn run(
    env: &mut Env,
    args: &Args,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let path = args.str(PATH)?;
    let old = args.str(OLD)?;
    let new = args.str(NEW)?;

    let p = env.workspace.resolve(&path, true)?;
    let text = std::fs::read_to_string(&p).context("failed to read file")?;

    match text.matches(&old).count() {
        0 => Err(anyhow!(
            "old text not found; it must match the file content exactly (including whitespace and newlines)"
        )),
        1 => {
            // The replaced run's first line, computed while the original is still here; it goes to
            // `memo` for the front end's diff gutter.
            let pos = text.find(&old).expect("count == 1 checked above");
            let at = line_of(&text, pos);
            let updated = text.replacen(&old, &new, 1);
            std::fs::write(&p, updated).context("failed to write file")?;
            Ok(ToolOutput::with_memo(
                format!("replaced: {}", p.display()),
                json!({ "kind": "diff", "atLine": at }),
            ))
        }
        n => Err(anyhow!(
            "old appears {n} times; edit requires a unique match. \
             Use mass_edit to replace by line number, or make old \
             longer with surrounding context so it is unique"
        )),
    }
}

/// Which line (1-based) a byte position sits on.
fn line_of(text: &str, pos: usize) -> usize {
    text[..pos.min(text.len())].matches('\n').count() + 1
}
