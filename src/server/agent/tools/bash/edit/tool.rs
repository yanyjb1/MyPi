//! The `edit` tool: replace one unique run of text, or one range of lines.

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace::write_atomic;

const TYPE: usize = 0;
const PATH: usize = 1;
const OLD: usize = 2;
const RANGE: usize = 3;
const NEW: usize = 4;

const PARAMS: &[Param] = &[
    Param::new(
        "type",
        Kind::Choice(&["exact", "mass"]),
        "Which edit this is.",
    ),
    Param::new("path", Kind::Text, "File path."),
    Param::new(
        "old",
        Kind::Text,
        "The text to replace. MUST match the file character for character, exactly once.",
    ),
    Param::new(
        "range",
        Kind::Text,
        "The lines to replace, `12-14` or `12`.",
    ),
    Param::new(
        "new",
        Kind::Text,
        "The replacement; any number of lines. Empty deletes the target.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "edit",
    about: "Edit one file. `type: exact` replaces one run of `old` with `new`. `type: mass` \
            replaces the lines named by `range` with `new`.",
    input: "type and path are always needed. The other type's field is refused.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let kind = args.choice(TYPE)?;
    let path = args.str(PATH)?;
    let p = env.workspace.resolve(&path, true)?;

    env.workspace.with_write(&p, || {
        let text = std::fs::read_to_string(&p).context("failed to read file")?;
        let (updated, memo, message) = if kind == "exact" {
            exact(&text, args)?
        } else {
            mass(&text, args)?
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
        args.opt_str(RANGE)?.is_none(),
        "`range` is only for type=mass"
    );
    exact_text(text, &args.str(OLD)?, &args.str(NEW)?)
}

fn mass(text: &str, args: &Args) -> Result<(String, Value, String)> {
    anyhow::ensure!(args.opt_str(OLD)?.is_none(), "`old` is only for type=exact");
    let (from, to) = parse_range(&args.str(RANGE)?)?;
    let block = args.str(NEW)?;
    mass_text(text, from, to, &block)
}

/// Replace one unique run of text.
pub(crate) fn exact_text(text: &str, old: &str, new: &str) -> Result<(String, Value, String)> {
    anyhow::ensure!(!old.is_empty(), "`old` must not be empty");

    match text.matches(old).count() {
        0 => Err(anyhow!("`old` not found in the file")),
        1 => {
            let pos = text.find(old).expect("count == 1 checked above");
            let at = line_of(text, pos);
            let old_lines = lines_of(old).len();
            let new_lines = lines_of(new).len();
            let total = text.lines().count() + new_lines - old_lines;
            Ok((
                text.replacen(old, new, 1),
                json!({ "kind": "diff", "atLine": at }),
                receipt(at, old_lines, new_lines, total),
            ))
        }
        n => Err(anyhow!(
            "`old` appears {n} times; make it longer with surrounding context, \
             or use type=mass to replace by line number"
        )),
    }
}

/// Replace whole lines by 1-based number: `from..=to` becomes `new`, any number of lines.
pub(crate) fn mass_text(
    text: &str,
    from: usize,
    to: usize,
    new: &str,
) -> Result<(String, Value, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    anyhow::ensure!(from >= 1, "`range` is 1-based; got {from}");
    anyhow::ensure!(from <= to, "`range` must count upwards; got {from}-{to}");
    anyhow::ensure!(
        to <= total,
        "line {to} does not exist; file has {total} lines"
    );

    let block: Vec<&str> = match new {
        "" => Vec::new(),
        // One trailing newline is the block's own terminator, not an extra empty line.
        text => text
            .strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .collect(),
    };
    let old_lines = to - from + 1;
    let new_lines = block.len();

    let mut out: Vec<&str> = Vec::with_capacity(total - old_lines + new_lines);
    out.extend_from_slice(&lines[..from - 1]);
    out.extend_from_slice(&block);
    out.extend_from_slice(&lines[to..]);
    let mut text_out = out.join("\n");
    if text.ends_with('\n') {
        text_out.push('\n');
    }

    let old_text = lines[from - 1..to].join("\n");
    let after = total - old_lines + new_lines;
    Ok((
        text_out,
        json!({ "kind": "diff", "old": old_text }),
        receipt(from, old_lines, new_lines, after),
    ))
}

/// `12-14` or `12`, 1-based, ascending.
pub(crate) fn parse_range(text: &str) -> Result<(usize, usize)> {
    let text = text.trim();
    let (from, to) = match text.split_once('-') {
        Some((from, to)) => (from.trim(), to.trim()),
        None => (text, text),
    };
    let number = |raw: &str| -> Result<usize> {
        raw.parse::<usize>()
            .map_err(|_| anyhow!("`range` must be `12-14` or `12`; got `{text}`"))
    };
    Ok((number(from)?, number(to)?))
}

/// The one line a model reads back: the lines that went, the lines that came, what moved below.
fn receipt(from: usize, old_lines: usize, new_lines: usize, total: usize) -> String {
    let to = from + old_lines.saturating_sub(1);
    let below = new_lines as i64 - old_lines as i64;
    format!(
        "replaced lines {from}-{to} ({old_lines}) with {new_lines} lines; below {below:+}; \
         file now {total} lines"
    )
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

/// One line for a history listing: which file.
fn summary(call: &CallRecord) -> String {
    summary::clip(&summary::field(call, "path").unwrap_or_default(), 96)
}
