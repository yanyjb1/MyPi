//! The `edit` tool: replace one unique run of text, or replace whole lines by number.

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};

use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace::write_atomic;
use crate::server::agent::tools::{
    Args, CallRecord, Env, Kind, Param, Tool, ToolOutput, ToolProgress,
};

const TYPE: usize = 0;
const PATH: usize = 1;
const OLD: usize = 2;
const NEW: usize = 3;
const EDITS: usize = 4;

/// One entry's shape in `edits`.
const RECORD: &[Param] = &[
    Param::req("line", Kind::Line, "Line number, 1-based."),
    Param::req("text", Kind::Text, "New content for that line."),
];
const RECORD_LINE: usize = 0;
const RECORD_TEXT: usize = 1;

const PARAMS: &[Param] = &[
    Param::req(
        "type",
        Kind::Choice(&["exact", "mass"]),
        "exact: replace one unique run of text; mass: replace whole lines by number.",
    ),
    Param::req("path", Kind::Text, "File path. `./` and `../` allowed."),
    Param::opt(
        "old",
        Kind::Text,
        "exact: text to replace. MUST match exactly and be unique.",
    ),
    Param::opt("new", Kind::Text, "exact: replacement text."),
    Param::opt(
        "edits",
        Kind::Table(RECORD),
        "mass: one entry per line to replace.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "edit",
    about: "Edit one file. `type: exact` replaces one run of text and requires `old` to match the file character for character exactly once. `type: mass` replaces whole lines by number and requires a non-empty `edits`. The other group's fields are rejected. Only files inside the workspace and the temp directory.",
    params: PARAMS,
    run,
    summary: Some(summary),
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let kind = args.choice(TYPE)?;
    let path = args.str(PATH)?;
    let p = env.workspace.resolve(&path, true)?;

    env.workspace.with_write(&p, || {
        let text = std::fs::read_to_string(&p).context("failed to read file")?;
        let (updated, memo, message) = match kind.as_str() {
            "exact" => exact(&text, args)?,
            "mass" => mass(&text, args)?,
            other => return Err(anyhow!("unknown edit type `{other}`")),
        };
        write_atomic(&p, &updated)?;
        Ok(ToolOutput::with_memo(
            format!("{message}: {}", p.display()),
            memo,
        ))
    })
}

fn exact(text: &str, args: &Args) -> Result<(String, Value, String)> {
    anyhow::ensure!(
        args.opt_any(EDITS).is_none(),
        "`edits` is only for type=mass"
    );
    exact_text(text, &args.str(OLD)?, &args.str(NEW)?)
}

fn mass(text: &str, args: &Args) -> Result<(String, Value, String)> {
    anyhow::ensure!(
        args.opt_str(OLD)?.is_none() && args.opt_str(NEW)?.is_none(),
        "`old`/`new` are only for type=exact"
    );
    let rows = args.table(EDITS)?;
    let mut edits = Vec::with_capacity(rows.len());
    for row in &rows {
        edits.push((row.line(RECORD_LINE)?, row.str(RECORD_TEXT)?));
    }
    mass_text(text, &edits)
}

/// Replace one unique run of text.
pub(crate) fn exact_text(text: &str, old: &str, new: &str) -> Result<(String, Value, String)> {
    anyhow::ensure!(!old.is_empty(), "`old` must not be empty");

    match text.matches(old).count() {
        0 => Err(anyhow!(
            "old text not found; it must match the file content exactly (including whitespace and newlines)"
        )),
        1 => {
            let pos = text.find(old).expect("count == 1 checked above");
            let at = line_of(text, pos);
            // The memo must survive the file: old text, new text, and the lines each sat on.
            let deletions = numbered(at, &lines_of(old));
            let insertions = numbered(at, &lines_of(new));
            Ok((
                text.replacen(old, new, 1),
                json!({
                    "kind": "diff",
                    "atLine": at,
                    "deletions": deletions,
                    "insertions": insertions,
                }),
                "replaced".to_string(),
            ))
        }
        n => Err(anyhow!(
            "old appears {n} times; edit requires a unique match. \
             Use type=mass to replace by line number, or make old \
             longer with surrounding context so it is unique"
        )),
    }
}

/// Replace whole lines by 1-based number.
pub(crate) fn mass_text(text: &str, edits: &[(usize, String)]) -> Result<(String, Value, String)> {
    anyhow::ensure!(!edits.is_empty(), "`edits` must not be empty");
    let total = text.lines().count();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut edits = edits.to_vec();
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));

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

    let mut deletions = Vec::with_capacity(edits.len());
    let mut insertions = Vec::with_capacity(edits.len());
    let mut at = usize::MAX;
    for (line, new) in edits {
        deletions.push(json!({ "line": line, "text": lines[line - 1].clone() }));
        lines[line - 1] = new;
        insertions.push(json!({ "line": line, "text": lines[line - 1].clone() }));
        at = at.min(line);
    }
    let changed = deletions.len();

    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    Ok((
        out,
        json!({
            "kind": "diff",
            "atLine": at,
            "deletions": deletions,
            "insertions": insertions,
        }),
        format!("replaced {changed} line(s)"),
    ))
}

/// Which line (1-based) a byte position sits on.
fn line_of(text: &str, pos: usize) -> usize {
    text[..pos.min(text.len())].matches('\n').count() + 1
}

/// A text's lines, with no empty line for the newline that ends it.
fn lines_of(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// The lines of one text, numbered from `first`.
fn numbered(first: usize, lines: &[String]) -> Vec<Value> {
    lines
        .iter()
        .enumerate()
        .map(|(at, text)| json!({ "line": first + at, "text": text }))
        .collect()
}

/// One line for a history listing: which file, and which lines when the call names them.
fn summary(call: &CallRecord) -> String {
    let path = summary::field(call, "path").unwrap_or_default();
    match edited_lines(call) {
        Some(lines) => summary::clip(&format!("{path} lines {lines}"), 96),
        None => summary::clip(&path, 96),
    }
}

/// The line numbers a `mass` call replaces, in order.
fn edited_lines(call: &CallRecord) -> Option<String> {
    let args: Value = serde_json::from_str(&call.args).ok()?;
    let mut lines = Vec::new();
    for entry in args.get("edits")?.as_array()? {
        lines.push(entry.get("line")?.as_i64()?.to_string());
    }
    (!lines.is_empty()).then(|| lines.join(","))
}
