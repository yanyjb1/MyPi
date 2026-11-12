//! The `context` tool: query the calls this session recorded, or show a call's request and result by block number.

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::{CallRecord, Direction};
use crate::server::agent::tools::registry;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const TYPE: usize = 0;
const ANCHOR: usize = 1;
const DIRECTION: usize = 2;
const LIMIT: usize = 3;
const TOOL_PARAM: usize = 4;
const IDS: usize = 5;

/// A listing longer than this goes to a file instead of the conversation.
const INLINE_LINES: usize = 10;
/// `show` writes its rows out past this many block numbers.
const INLINE_BLOCKS: usize = 3;

const PARAMS: &[Param] = &[
    Param::new(
        "type",
        Kind::Choice(&["query", "show"]),
        "Which operation this is.",
    ),
    Param::new(
        "anchor",
        Kind::Text,
        "A `#<ref>` hash of a truncated result (e.g. `#9c4c`), or a block id. Never a line number.",
    ),
    Param::df(
        "direction",
        Kind::Choice(&["before", "after"]),
        "Which way from the anchor.",
        "after",
    ),
    Param::df("limit", Kind::Number, "How many calls to return.", "3"),
    Param::df(
        "tool",
        Kind::Text,
        "One tool name, or `all` for every kind.",
        "bash",
    ),
    Param::new("ids", Kind::Numbers, "Block ids, as `query` reports them."),
];

pub(crate) const TOOL: Tool = Tool {
    name: "context",
    about: "Look at what this session has already recorded; nothing is executed. `query` walks the \
            calls around an anchor in one direction, for one tool or all of them, and answers one \
            JSON row per call: id, tool, ok, summary. `show` answers each block's request arguments \
            and result as JSON. A listing past ten calls, and a `show` past three block numbers, is \
            written to a file and the reply names that file.",
    fields: "type is always needed. query: anchor and direction, plus limit and tool to narrow \
            it. show: ids, one or more block ids; the request arguments and result come back as \
            JSON.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    match args.choice(TYPE)?.as_str() {
        "query" => query(env, args),
        "show" => show(env, args),
        other => Err(anyhow!("unknown context type `{other}`")),
    }
}

fn query(env: &Env, args: &Args) -> Result<ToolOutput> {
    let anchor = args
        .opt_str(ANCHOR)?
        .map(|anchor| anchor.trim().to_string())
        .filter(|anchor| !anchor.is_empty())
        .ok_or_else(|| {
            anyhow!("missing argument `anchor` for type=query (a `#<ref>` hash or a block id)")
        })?;
    let direction = match args.choice(DIRECTION)?.as_str() {
        "before" => Direction::Before,
        _ => Direction::After,
    };
    let limit = args.int(LIMIT)?.max(1) as usize;
    let tool = args.str(TOOL_PARAM)?;
    let scope = (tool != "all").then(|| tool.as_str());

    let records = env
        .source
        .history(env.session, &anchor, direction, limit, scope)?;
    if records.is_empty() {
        let which = scope.map_or("any", |t| t);
        return Ok(ToolOutput::text(format!(
            "(no {which} call {} {anchor})",
            direction_name(direction)
        )));
    }

    let memo = Value::Array(records.iter().map(listing_row).collect());
    let text = serde_json::to_string_pretty(&memo)?;
    if records.len() <= INLINE_LINES {
        return Ok(ToolOutput::with_memo(text, memo));
    }
    let session = env.session.to_string();
    let file = name(&[
        &session,
        "query",
        &anchor,
        direction_name(direction),
        &limit.to_string(),
        &tool,
    ]);
    let path = spill(env, &file, &text)?;
    Ok(ToolOutput::with_memo(
        format!(
            "written: {} ({} calls, {})",
            path.display(),
            records.len(),
            block_range(&records)
        ),
        memo,
    ))
}

fn show(env: &Env, args: &Args) -> Result<ToolOutput> {
    let ids = block_ids(args)?;
    anyhow::ensure!(!ids.is_empty(), "missing argument `ids` for type=show");

    let mut rows = Vec::with_capacity(ids.len());
    for id in &ids {
        rows.push(match env.source.block(env.session, *id)? {
            Some(pair) => json!({
                "id": id,
                "request": parsed(pair.request.get("args")),
                "result": parsed(pair.result.as_ref().and_then(|row| row.get("result"))),
            }),
            None => json!({ "id": id, "request": null, "result": null }),
        });
    }

    let memo = Value::Array(rows);
    let text = serde_json::to_string_pretty(&memo)?;
    if ids.len() <= INLINE_BLOCKS {
        return Ok(ToolOutput::with_memo(text, memo));
    }
    let session = env.session.to_string();
    let joined = ids.iter().map(i64::to_string).collect::<Vec<_>>().join("-");
    let path = spill(env, &name(&[&session, "show", &joined]), &text)?;
    Ok(ToolOutput::with_memo(
        format!("written: {} ({} blocks)", path.display(), ids.len()),
        memo,
    ))
}

/// One listing row: block number, tool, verdict, summary.
fn listing_row(record: &CallRecord) -> Value {
    json!({
        "id": record.block,
        "tool": &record.tool,
        "ok": record.ok,
        "summary": summary_of(record),
    })
}

/// A stored text field as JSON when it parses, else as the text itself.
fn parsed(text: Option<&Value>) -> Value {
    match text.and_then(Value::as_str) {
        Some(text) => serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.into())),
        None => Value::Null,
    }
}

/// The tool's own summary; a name the table no longer holds shows its arguments.
fn summary_of(record: &CallRecord) -> String {
    match registry::spec(&record.tool) {
        Some(tool) => (tool.summary)(record),
        None => summary::clip(&record.args, 96),
    }
}

fn block_range(records: &[CallRecord]) -> String {
    match (records.first(), records.last()) {
        (Some(first), Some(last)) => format!("{}..{}", first.block, last.block),
        _ => String::new(),
    }
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Before => "before",
        Direction::After => "after",
    }
}

/// Block numbers as given: integers, or their decimal spelling.
fn block_ids(args: &Args) -> Result<Vec<i64>> {
    let Some(value) = args.opt_any(IDS) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| anyhow!("`ids` must be an array of block numbers"))?;
    let mut ids = Vec::with_capacity(array.len());
    for one in array {
        let id = match one {
            serde_json::Value::Number(n) => n.as_i64(),
            serde_json::Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
        .ok_or_else(|| anyhow!("`ids` takes block numbers; got {one}"))?;
        ids.push(id);
    }
    Ok(ids)
}

/// A deterministic file name: same arguments, same file, overwritten each time.
fn name(parts: &[&str]) -> String {
    let slug: Vec<String> = parts
        .iter()
        .map(|part| {
            let raw: String = part
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            raw.trim_matches('-').to_string()
        })
        .collect();
    format!("context-{}.txt", slug.join("-"))
}

fn spill(env: &Env, file: &str, text: &str) -> Result<std::path::PathBuf> {
    let dir = env.export.dir();
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let path = dir.join(file);
    env.workspace
        .with_write(&path, || {
            crate::server::agent::tools::workspace::write_atomic(&path, text)
        })
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

/// One line for a history listing: the operation.
fn summary(call: &CallRecord) -> String {
    summary::field(call, "type").unwrap_or_default()
}
