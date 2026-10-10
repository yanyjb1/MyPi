//! mass_edit: replace whole lines by line number. No searching, no matching.

use anyhow::{Context as _, Result, anyhow};
use serde_json::json;

use super::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

const PATH: usize = 0;
const EDITS: usize = 1;

/// One entry's shape in `edits`.
const RECORD: &[Param] = &[
    Param::req("line", Kind::Line, "Line number, 1-based."),
    Param::req("text", Kind::Text, "New content for that line."),
];
const RECORD_LINE: usize = 0;
const RECORD_TEXT: usize = 1;

pub(super) const PARAMS: &[Param] = &[
    Param::req("path", Kind::Text, "File path."),
    Param::req(
        "edits",
        Kind::Table(RECORD),
        "One entry per line to replace.",
    ),
];

pub(super) const TOOL: Tool = Tool {
    name: "mass_edit",
    about: "Replace whole lines by line number. No searching, no matching: you name the lines and they become the new\n\
            text. For multi-line rewrites, or when identical lines make `edit` unable to land uniquely.\n\
            Only files inside the workspace and /tmp.",
    params: PARAMS,
    run,
};

/// Replace whole lines by number, largest number first, so no line number shifts under a later
/// edit.
fn run(
    env: &mut Env,
    args: &Args,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let path = args.str(PATH)?;
    let rows = args.table(EDITS)?;

    let p = env.workspace.resolve(&path, true)?;
    let text = std::fs::read_to_string(&p).context("failed to read file")?;
    let total = text.lines().count();
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

    let mut edits: Vec<(usize, String)> = Vec::with_capacity(rows.len());
    for row in &rows {
        edits.push((row.line(RECORD_LINE)?, row.str(RECORD_TEXT)?));
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0)); // largest first

    // The same line twice is self-contradictory; refusing beats silently taking the last one.
    for w in edits.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(anyhow!("line {} was specified twice", w[0].0));
        }
    }
    if let Some(e) = edits.first().filter(|e| e.0 > total) {
        return Err(anyhow!(
            "line {} does not exist; file has {total} lines",
            e.0
        ));
    }

    // The replaced lines' originals, visible only here; they go to `memo` for the front end — the
    // one channel for fields the front end cannot compute. Ascending by line, ready to draw.
    let mut originals = Vec::with_capacity(edits.len());
    for (line, new) in edits {
        originals.push(json!({
            "line": line,
            "text": lines[line - 1].clone(),
        }));
        lines[line - 1] = new;
    }
    originals.reverse();
    let changed = originals.len();

    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&p, out).context("failed to write file")?;

    Ok(ToolOutput::with_memo(
        format!("replaced {changed} line(s) in {}", p.display()),
        json!({ "kind": "diff", "deletions": originals }),
    ))
}
