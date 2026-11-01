//! The `context` tool: query the calls this session recorded, or show whole rows by block number.

use anyhow::{Context as _, Result, anyhow};

use crate::server::agent::tools::contract::env::{CallRecord, Direction, Env};
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::registry;
use crate::server::agent::tools::summary;
use crate::server::agent::tools::{Args, Kind, Param, Tool, ToolProgress};

const OP: usize = 0;
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
    Param::req(
        "op",
        Kind::Choice(&["query", "show"]),
        "query: the calls around an anchor; show: whole rows by block number.",
    ),
    Param::opt(
        "anchor",
        Kind::Text,
        "query: a spilled-text reference (`#<ref>`) or a block number.",
    ),
    Param::opt(
        "direction",
        Kind::Choice(&["back", "forward"]),
        "query: which way from the anchor.",
    ),
    Param::df(
        "limit",
        Kind::Number,
        "query: how many calls to return.",
        "3",
    ),
    Param::df(
        "tool",
        Kind::Text,
        "query: one tool name, or `all` for every kind.",
        "bash",
    ),
    Param::opt("ids", Kind::Numbers, "show: one or more block numbers."),
];

pub(crate) const TOOL: Tool = Tool {
    name: "context",
    about: "Look at what this session has already recorded; nothing is executed. `query` walks the calls\n\
            around an anchor — an artifact reference `#<ref>` or a block number — in one direction, for one\n\
            tool or all of them, and answers one line per call: block, tool, ok, summary. `show` returns the\n\
            whole request and result rows as JSON for the block numbers given. A listing past ten lines, and a\n\
            `show` past three block numbers, is written to a file and the reply names that file.",
    params: PARAMS,
    run,
    summary: Some(summary),
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    match args.choice(OP)?.as_str() {
        "query" => query(env, args),
        "show" => show(env, args),
        other => Err(anyhow!("unknown context op `{other}`")),
    }
}

fn query(env: &Env, args: &Args) -> Result<ToolOutput> {
    let anchor = args.str(ANCHOR)?.trim().to_owned();
    anyhow::ensure!(!anchor.is_empty(), "`anchor` must not be empty");
    let direction = match args.choice(DIRECTION)?.as_str() {
        "back" => Direction::Back,
        _ => Direction::Forward,
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

    let lines: Vec<String> = records.iter().map(render).collect();
    let text = lines.join("\n");
    if lines.len() <= INLINE_LINES {
        return Ok(ToolOutput::text(text));
    }
    let file = name(&[
        "query",
        &anchor,
        direction_name(direction),
        &limit.to_string(),
        &tool,
    ]);
    let path = spill(env, &file, &text)?;
    Ok(ToolOutput::text(format!(
        "written: {} ({} calls, {})",
        path.display(),
        records.len(),
        block_range(&records)
    )))
}

fn show(env: &Env, args: &Args) -> Result<ToolOutput> {
    let ids = block_ids(args)?;
    anyhow::ensure!(!ids.is_empty(), "`ids` is required by `show`");

    let mut lines = Vec::with_capacity(ids.len() * 2);
    for id in &ids {
        match env.source.block(env.session, *id)? {
            Some(pair) => {
                lines.push(format!("{id} request: {}", one_line(&pair.request)));
                let result = pair
                    .result
                    .map_or_else(|| "(none recorded)".to_string(), |v| one_line(&v));
                lines.push(format!("{id} result: {result}"));
            }
            None => lines.push(format!("{id} (no such block in this session)")),
        }
    }

    let text = lines.join("\n");
    if ids.len() <= INLINE_BLOCKS {
        return Ok(ToolOutput::text(text));
    }
    let joined = ids.iter().map(i64::to_string).collect::<Vec<_>>().join("-");
    let path = spill(env, &name(&["show", &joined]), &text)?;
    Ok(ToolOutput::text(format!(
        "written: {} ({} blocks)",
        path.display(),
        ids.len()
    )))
}

/// One listing line: block number, tool, verdict, summary.
fn render(record: &CallRecord) -> String {
    format!(
        "{} {} {}: {}",
        record.block,
        record.tool,
        if record.ok { "ok" } else { "failed" },
        summary_of(record)
    )
}

/// The tool's own summary; a tool without one falls back to its arguments.
fn summary_of(record: &CallRecord) -> String {
    match registry::spec(&record.tool).and_then(|tool| tool.summary) {
        Some(summary) => summary(record),
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
        Direction::Back => "back",
        Direction::Forward => "forward",
    }
}

/// One JSON row on one line.
fn one_line(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
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
    std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

/// One line for a history listing: the operation and what it looked at.
fn summary(call: &CallRecord) -> String {
    let op = summary::field(call, "op").unwrap_or_default();
    if op == "show" {
        return summary::clip(&format!("show {}", block_list(call)), 96);
    }
    let tool = summary::field(call, "tool").unwrap_or_else(|| "bash".to_string());
    let direction = summary::field(call, "direction").unwrap_or_default();
    let anchor = summary::field(call, "anchor").unwrap_or_default();
    summary::clip(&format!("query {tool} {direction} from {anchor}"), 96)
}

fn block_list(call: &CallRecord) -> String {
    let args: serde_json::Value = match serde_json::from_str(&call.args) {
        Ok(args) => args,
        Err(_) => return String::new(),
    };
    let Some(ids) = args.get("ids").and_then(|v| v.as_array()) else {
        return String::new();
    };
    ids.iter()
        .filter_map(|v| v.as_i64())
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
