//! The static `mcp` tool: list configured servers/tools or call one tool.

pub(crate) mod bridge;
pub(crate) mod setup;
mod stdio;
pub(crate) mod sync;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::contract::env::Env;
use super::contract::output::ToolOutput;
use super::contract::view::CallRecord;
use super::spec::{Args, Kind, Param, Tool, ToolProgress};
use super::summary;

use bridge::{McpBridge, McpContent, McpResult};

const TYPE: usize = 0;
const SERVER: usize = 1;
const TOOL_PARAM: usize = 2;
const ARGS: usize = 3;

const PARAMS: &[Param] = &[
    Param::new(
        "type",
        Kind::Choice(&["list", "call"]),
        "Which operation this is.",
    ),
    Param::new("server", Kind::Text, "Server name."),
    Param::new("tool", Kind::Text, "Tool name, as `list` prints it."),
    Param::new(
        "args",
        Kind::Object,
        "One key per parameter `list` prints for that tool; omit it when the tool declares none.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "mcp",
    about: "MCP servers and their tools. `list` names the servers and their tools; naming one \
            server also prints each tool's parameters. `call` runs one tool with `args` of that \
            shape.",
    input: "type is always needed. list: server prints that server's tools and their parameters; \
            omit it for the servers and their tool lists. call: server and tool, plus args built \
            from the parameters `list` printed.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let bridge = env.source.mcp(env.session, &env.workspace.cwd())?;
    match args.choice(TYPE)?.as_str() {
        "list" => {
            let server = args.opt_str(SERVER)?;
            list(bridge.as_ref(), server.as_deref())
        }
        "call" => call(bridge.as_ref(), args),
        _ => unreachable!("Args::choice validates type"),
    }
}

fn list(bridge: &dyn McpBridge, server: Option<&str>) -> Result<ToolOutput> {
    let mut entries = Vec::new();
    match server.map(str::trim).filter(|server| !server.is_empty()) {
        Some(server) => entries.push(server_json(bridge, server, true)),
        None => {
            for server in bridge.servers() {
                entries.push(server_json(bridge, &server.name, false));
            }
        }
    }
    let memo = Value::Array(entries);
    let text = serde_json::to_string_pretty(&memo)?;
    Ok(ToolOutput::with_memo(text, memo))
}

/// One server's row; `full` adds each tool's declared parameters, the index over every server
/// leaves them out.
fn server_json(bridge: &dyn McpBridge, server: &str, full: bool) -> Value {
    match bridge.tools(server) {
        Ok(tools) => json!({
            "server": server,
            "tools": tools
                .iter()
                .map(|tool| {
                    if full {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "required": tool.required,
                            "parameters": tool.parameters,
                        })
                    } else {
                        json!({"name": tool.name, "description": tool.description})
                    }
                })
                .collect::<Vec<_>>(),
        }),
        Err(error) => json!({"server": server, "error": format!("{error:#}")}),
    }
}

fn call(bridge: &dyn McpBridge, args: &Args) -> Result<ToolOutput> {
    let server = args.opt_str(SERVER)?;
    let server = server
        .as_deref()
        .map(str::trim)
        .filter(|server| !server.is_empty())
        .ok_or_else(|| anyhow!("missing argument `server` for type=call"))?;
    let tool = args.opt_str(TOOL_PARAM)?;
    let tool = tool
        .as_deref()
        .map(str::trim)
        .filter(|tool| !tool.is_empty())
        .ok_or_else(|| anyhow!("missing argument `tool` for type=call"))?;
    let arguments = match args.opt_any(ARGS) {
        None => json!({}),
        Some(value) if value.is_null() => json!({}),
        Some(value) if value.is_object() => value.clone(),
        Some(_) => bail!("`args` must be an object: one key per parameter the tool declares"),
    };
    let result = bridge.call(&server, &tool, arguments)?;
    let text = render(&result);
    if result.is_error {
        if text.is_empty() {
            bail!("MCP tool returned an error");
        }
        bail!("{text}");
    }
    Ok(ToolOutput::text(text))
}

fn render(result: &McpResult) -> String {
    if result.content.is_empty() {
        return "(no content)".to_string();
    }
    let mut out = String::new();
    for content in &result.content {
        if !out.is_empty() {
            out.push('\n');
        }
        match content {
            McpContent::Text(text) => out.push_str(text),
            McpContent::Unsupported { kind } => {
                out.push_str(&format!("[unsupported content: {kind}]"));
            }
        }
    }
    out
}

/// One line for a history listing: the operation.
fn summary(call: &CallRecord) -> String {
    let op = summary::field(call, "type").unwrap_or_default();
    summary::clip(&op, 96)
}
