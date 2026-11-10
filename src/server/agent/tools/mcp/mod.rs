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

const PARAMS: &[Param] = &[
    Param::new(
        "type",
        Kind::Choice(&["list", "call"]),
        "Which operation this is.",
    ),
    Param::new("server", Kind::Text, "Server name."),
    Param::new("tool", Kind::Text, "Tool name."),
    Param::new(
        "args",
        Kind::Object,
        "The tool's arguments, as an object; omit it when the tool takes none.",
    ),
];

pub const TOOL: Tool = Tool {
    name: "mcp",
    about: "MCP: list configured servers and tools, or call one tool.",
    input: "type is always needed. list: server narrows it to one; omit server for all. \
            call: server and tool, plus args when that tool takes any.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let Some(bridge) = env.mcp.as_deref() else {
        bail!("mcp is not configured");
    };
    let op = args.choice(0)?;
    match op.as_str() {
        "list" => list(bridge, args.opt_str(1)?),
        "call" => call(bridge, args),
        _ => unreachable!("Args::choice validates op"),
    }
}

fn list(bridge: &dyn McpBridge, server: Option<String>) -> Result<ToolOutput> {
    let mut entries = Vec::new();
    match server.filter(|server| !server.trim().is_empty()) {
        Some(server) => entries.push(server_json(bridge, &server)),
        None => {
            for server in bridge.servers() {
                entries.push(server_json(bridge, &server.name));
            }
        }
    }
    let memo = Value::Array(entries);
    let text = serde_json::to_string_pretty(&memo)?;
    Ok(ToolOutput::with_memo(text, memo))
}

fn server_json(bridge: &dyn McpBridge, server: &str) -> Value {
    match bridge.tools(server) {
        Ok(tools) => json!({
            "server": server,
            "tools": tools
                .iter()
                .map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "required": tool.required,
                }))
                .collect::<Vec<_>>(),
        }),
        Err(error) => json!({"server": server, "error": format!("{error:#}")}),
    }
}

fn call(bridge: &dyn McpBridge, args: &Args) -> Result<ToolOutput> {
    let server = args
        .opt_str(1)?
        .filter(|server| !server.trim().is_empty())
        .ok_or_else(|| anyhow!("type=call needs `server`"))?;
    let tool = args
        .opt_str(2)?
        .filter(|tool| !tool.trim().is_empty())
        .ok_or_else(|| anyhow!("type=call needs `tool`"))?;
    let arguments = match args.opt_any(3) {
        None => json!({}),
        Some(value) if value.is_null() => json!({}),
        Some(value) if value.is_object() => value.clone(),
        Some(_) => bail!("`args` must be an object holding the tool's arguments, not a string"),
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
